"""One configured text account -> an independent package -> the unchanged public SDK.

No vendor dispatch, native API fallback, automatic retry or key in the package/receipt.
Windows credentials use an owner/SYSTEM-only, OS delete-on-close temporary file.
"""
import argparse
import base64
from contextlib import contextmanager
import csv
import ctypes
from ctypes import wintypes
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import time
from urllib.parse import urlsplit
import uuid
from cryptography.hazmat.primitives.ciphers.aead import AESGCM


@contextmanager
def private_credential(root, key):
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    advapi = ctypes.WinDLL('advapi32', use_last_error=True)
    class SecurityAttributes(ctypes.Structure):
        _fields_ = [('length', wintypes.DWORD), ('descriptor', wintypes.LPVOID), ('inherit', wintypes.BOOL)]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(wintypes.LPVOID), ctypes.POINTER(wintypes.DWORD)]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.restype = wintypes.BOOL
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(SecurityAttributes), wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.WriteFile.argtypes = [wintypes.HANDLE, wintypes.LPCVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
    kernel.WriteFile.restype = wintypes.BOOL
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.LocalFree.argtypes = [wintypes.HLOCAL]
    sid = list(csv.reader(subprocess.check_output(['whoami', '/user', '/fo', 'csv', '/nh'], text=True).splitlines()))[0][1]
    descriptor = wintypes.LPVOID()
    assert advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW('D:P(A;;GA;;;' + sid + ')(A;;GA;;;SY)', 1, ctypes.byref(descriptor), None)
    attributes = SecurityAttributes(ctypes.sizeof(SecurityAttributes), descriptor, False)
    path = root / ('credential-' + uuid.uuid4().hex + '.tmp')
    handle = kernel.CreateFileW(str(path), 0xC0010000, 7, ctypes.byref(attributes), 1, 0x04000100, None)
    kernel.LocalFree(descriptor)
    assert handle != wintypes.HANDLE(-1).value, 'Private credential channel creation failed'
    buffer = ctypes.create_string_buffer(key)
    try:
        written = wintypes.DWORD()
        assert kernel.WriteFile(handle, buffer, len(key), ctypes.byref(written), None) and written.value == len(key)
        ctypes.memset(buffer, 0, len(buffer))
        yield path
    finally:
        ctypes.memset(buffer, 0, len(buffer))
        kernel.CloseHandle(handle)
        assert not path.exists(), 'Temporary credential was not removed'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['cli', 'sdk', 'worker', 'package', 'app-data', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--provider-kind', required=True)
    parser.add_argument('--model-id', required=True)
    parser.add_argument('--operation-id')
    parser.add_argument('--parameters', default='{}')
    parser.add_argument('--hold-for-app', action='store_true', help='Provide a temporary credential lease for an owned GUI acceptance; stop via OUTPUT/stop or after 15 minutes')
    args = parser.parse_args()
    assert all(value.is_absolute() for value in vars(args).values() if isinstance(value, Path))
    parameters = json.loads(args.parameters)
    assert isinstance(parameters, dict)
    db = sqlite3.connect('file:' + (args.app_data / 'story.db').as_posix() + '?mode=ro', uri=True)
    settings = json.loads(db.execute('SELECT payload FROM app_user_settings LIMIT 1').fetchone()[0]); db.close()
    bindings = [binding for binding in settings['capabilityBindings'] if binding.get('providerKind') == args.provider_kind
                and any(model.get('modelId') == args.model_id for model in binding.get('models', []))]
    assert len(bindings) == 1, 'Select exactly one existing configured account/model'
    account = next(item for item in settings['credentials'] if item['id'] == bindings[0]['credentialId'])
    url = urlsplit(account['baseUrl'])
    assert url.scheme == 'https' and url.hostname and not url.username and not url.password and not url.query and not url.fragment
    origin = url.scheme + '://' + url.netloc
    encrypted = account['apiKey']; assert encrypted, 'Selected account has no key'
    if encrypted.startswith('enc:v1:'):
        sealed = base64.b64decode(encrypted[7:])
        master = (args.app_data / '.bgxiong_credential_master_key').read_bytes(); assert len(master) == 32
        key = AESGCM(master).decrypt(sealed[:12], sealed[12:], None)
    else:
        key = encrypted.encode()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((args.package / 'manifest.json').read_text(encoding='utf-8'))
    budget = manifest['limits']['wallMs']
    workspace = args.output / 'workspace'
    record = {'pluginId': manifest['pluginId'], 'version': manifest['version'], 'modelId': args.model_id, 'origin': origin,
              'packageSha256': hashlib.sha256((args.package / manifest['entry']).read_bytes()).hexdigest(),
              'cliSha256': hashlib.sha256(args.cli.read_bytes()).hexdigest(), 'workerSha256': hashlib.sha256(args.worker.read_bytes()).hexdigest(),
              'providerKindUsedOnlyForCredentialLookup': args.provider_kind, 'credentialStoredInReceipt': False, 'automaticRetries': 0}
    try:
        with private_credential(args.output, key) as secret_path:
            config = {'bindingId': 'configured-text', 'bindingRevision': 1, 'configurationRef': 'configured-text-v1',
                      'configuration': {'endpointBaseUrl': account['baseUrl'].rstrip('/'), 'credentialRef': 'configured-account'},
                      'trust': {'publishers': [], 'revokedPlugins': [], 'revokedVersions': [], 'revokedKeys': [], 'denylistedHosts': [], 'isFresh': False, 'supportedFeatures': []},
                      'network': {'origins': [origin], 'allowPrivate': False, 'allowEmbeddedAuthorization': False, 'deniedHosts': [], 'deniedNetworks': []},
                      'credentials': [{'reference': 'configured-account', 'secretFile': str(secret_path), 'publicId': None, 'customHeader': None, 'modes': ['bearer'], 'origins': [origin], 'ttlMs': budget}],
                      'media': [], 'mediaTtlMs': budget, 'transport': {'handshakeTimeoutMs': 10000, 'writeTimeoutMs': budget, 'shutdownTimeoutMs': 10000, 'cancelGraceMs': 100}, 'probe': None}
            config_path = args.output / 'host-config.json'; config_path.write_text(json.dumps(config), encoding='utf-8')
            if args.hold_for_app:
                app_config = dict(config)
                # Installation pins connection configuration; media is provisioned per call.
                del app_config['media']
                app_config['budgetMs'] = {operation: budget for operation in ['capabilities', 'submit', 'query', 'cancel', 'probe']}
                app_path = args.output / 'host-config.app.json'
                app_path.write_text(json.dumps(app_config, ensure_ascii=False, indent=2), encoding='utf-8')
                (args.output / 'ready.json').write_text(json.dumps({'hostConfig': str(app_path), 'package': str(args.package), 'modelId': args.model_id, 'deadlineSeconds': 900}), encoding='utf-8')
                print(json.dumps({'status': 'CREDENTIAL_LEASE_READY', 'hostConfig': str(app_path), 'deadlineSeconds': 900}), flush=True)
                deadline = time.monotonic() + 900
                while time.monotonic() < deadline and not (args.output / 'stop').exists():
                    time.sleep(0.25)
                record['status'] = 'CREDENTIAL_LEASE_CLOSED'
                return
            def invoke(operation, payload):
                request = {'protocolVersion': 2, 'operation': operation, 'profileId': config['bindingId'], 'bindingId': config['bindingId'],
                           'capabilitySlot': 'generic' if operation == 'capabilities' else 'text', 'requestId': 'text.' + uuid.uuid4().hex,
                           'budgetMs': budget, 'payloadJson': json.dumps(payload, ensure_ascii=False), 'negotiatedVersion': 2, 'enabledFeatures': []}
                source = args.output / (operation + '.input.json'); source.write_text(json.dumps(request, ensure_ascii=False, indent=2), encoding='utf-8')
                target = workspace / 'receipts' / (operation + '.output.json')
                command = [str(args.cli), operation, '--sdk', str(args.sdk), '--worker', str(args.worker), '--package', str(args.package), '--workspace', str(workspace),
                           '--config', str(config_path), '--input', str(source), '--output', str(target)]
                if not workspace.exists(): command.append('--create-workspace')
                run = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', timeout=budget / 1000 + 30)
                for channel in ['stdout', 'stderr']:
                    (args.output / (operation + '.' + channel + '.log')).write_text(getattr(run, channel).replace(key.decode(), '[REDACTED]'), encoding='utf-8')
                response = json.loads(target.read_text(encoding='utf-8')) if target.exists() else None
                record[operation] = {'command': command, 'requestId': request['requestId'], 'exitCode': run.returncode, 'response': response}
                assert run.returncode == 0 and response and 'dataJson' in response, 'Public SDK operation failed; no native fallback or retry'
                return json.loads(response['dataJson'])
            catalog = invoke('capabilities', {})
            choices = [(contract, operation) for contract in catalog['capabilityContracts'] if contract['capabilityKey'] == 'text.generate' and contract['transportSlot'] == 'text' and args.model_id in contract['modelIds']
                       for operation in contract['operations'] if args.operation_id is None or operation['id'] == args.operation_id]
            assert len(choices) == 1, 'An unambiguous declared text operation is required'
            contract, operation = choices[0]
            result = invoke('submit', {'schemaVersion': 2, 'modelId': args.model_id, 'capabilityKey': contract['capabilityKey'], 'operationId': operation['id'], 'operationVersion': operation['version'],
                                      'inputSchemaDigest': operation['inputSchema']['digest'], 'input': {'messages': [{'role': 'user', 'content': '请用一句中文说明你已收到文本接口测试请求，不要调用工具。'}], 'parameters': parameters},
                                      'configuration': config['configuration'], 'media': [], 'uploads': []})
            assert result['kind'] == 'completed' and len(result['outputs']) == 1 and result['outputs'][0]['kind'] == 'text' and result['outputs'][0]['role'] == 'answer'
            assert isinstance(result['outputs'][0]['text'], str) and result['outputs'][0]['text']
            record['status'] = 'PASS'
    finally:
        record['temporaryCredentialRemoved'] = not any(args.output.glob('credential-*.tmp'))
        (args.output / 'result.json').write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'status': record['status'], 'pluginId': manifest['pluginId'], 'modelId': args.model_id, 'text': result['outputs'][0]['text'], 'reportedUsage': result['usage'], 'receipt': str(args.output / 'result.json')}, ensure_ascii=False))

if __name__ == '__main__': main()
