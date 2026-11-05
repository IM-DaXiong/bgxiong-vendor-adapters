"""One real M3 call through the public SDK, using an existing app credential read-only.

Windows: the SDK receives a private delete-on-close credential file, removed after the call.
No retries. Receipts contain the nonsecret request, SDK result and reported usage only.
"""
import argparse
import base64
import csv
import ctypes
from ctypes import wintypes
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
from urllib.parse import urlsplit
import uuid
import urllib.request
import urllib.error
from cryptography.hazmat.primitives.ciphers.aead import AESGCM


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("cli", "sdk", "worker", "package", "app-data", "output", "catalog-receipt"):
        parser.add_argument("--" + field, required=True, type=Path)
    parser.add_argument('--transport', choices=['sdk', 'direct-baseline'], default='sdk')
    args = parser.parse_args()
    if any(not value.is_absolute() for value in vars(args).values() if isinstance(value, Path)):
        parser.error("All paths must be absolute")
    connection = sqlite3.connect('file:' + (args.app_data / 'story.db').as_posix() + '?mode=ro', uri=True)
    settings = json.loads(connection.execute('select payload from app_user_settings limit 1').fetchone()[0])
    connection.close()
    bindings = [x for x in settings.get('capabilityBindings', []) if x.get('providerKind') == 'text_minimax_m3' and any(m.get('modelId') == 'MiniMax-M3' for m in x.get('models', []))]
    assert len(bindings) == 1, 'Expected one configured M3 account selection'
    credential = next(x for x in settings['credentials'] if x['id'] == bindings[0]['credentialId'])
    parsed = urlsplit(credential['baseUrl'])
    assert parsed.scheme == 'https' and parsed.hostname in {'api.minimaxi.com', 'api.minimax.cn', 'api.minimax.io'} and parsed.path in {'', '/'}, 'Use the exact configured official origin'
    origin = parsed.scheme + '://' + parsed.netloc
    encrypted = credential['apiKey']
    assert encrypted, 'Configured M3 account has no API key'
    if encrypted.startswith('enc:v1:'):
        sealed = base64.b64decode(encrypted[7:])
        master = (args.app_data / '.bgxiong_credential_master_key').read_bytes()
        assert len(master) == 32
        key = AESGCM(master).decrypt(sealed[:12], sealed[12:], None)
    else:
        key = encrypted.encode()
    assert key, 'Configured credential is empty'
    args.output.mkdir(parents=True, exist_ok=False)

    if args.transport == 'direct-baseline':
        # Diagnostic of the configured account/network only; this is NOT an SDK pass.
        body = {'model': 'MiniMax-M3', 'messages': [{'role': 'user', 'content': '请用一句中文说明你已收到文本接口测试请求，不要调用工具。'}],
                'thinking': {'type': 'disabled'}, 'reasoning_split': True, 'max_completion_tokens': 128, 'stream': False}
        request = urllib.request.Request(origin + '/v1/chat/completions', data=json.dumps(body, ensure_ascii=False).encode(),
                                         headers={'Authorization': 'Bearer ' + key.decode(), 'Content-Type': 'application/json'}, method='POST')
        (args.output / 'request.json').write_text(json.dumps({'origin': origin, 'body': body}, ensure_ascii=False, indent=2), encoding='utf-8')
        try:
            with urllib.request.urlopen(request, timeout=180) as reply:
                status, raw = reply.status, reply.read()
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read()
        except Exception as error:
            (args.output / 'result.json').write_text(json.dumps({'transport': 'direct-baseline', 'sdkPass': False, 'errorType': type(error).__name__, 'noRetry': True}), encoding='utf-8')
            raise RuntimeError('Direct account/network baseline failed; no retry') from None
        value = json.loads(raw)
        receipt = {'transport': 'direct-baseline', 'sdkPass': False, 'httpStatus': status, 'origin': origin, 'noRetry': True, 'response': value}
        (args.output / 'result.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2), encoding='utf-8')
        assert status == 200 and value.get('model') == 'MiniMax-M3', 'Configured account baseline failed; inspect structured receipt'
        assert isinstance(value['choices'][0]['message']['content'], str) and value['choices'][0]['message']['content']
        assert value.get('base_resp', {}).get('status_code', 0) == 0
        print(json.dumps({'status': 'PASS', 'transport': 'direct-baseline', 'sdkPass': False, 'text': value['choices'][0]['message']['content'], 'usage': value.get('usage'), 'receipt': str(args.output / 'result.json')}, ensure_ascii=False))
        return

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
    kernel.FlushFileBuffers.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.LocalFree.argtypes = [wintypes.HLOCAL]
    sid = list(csv.reader(subprocess.check_output(['whoami', '/user', '/fo', 'csv', '/nh'], text=True).splitlines()))[0][1]
    descriptor = wintypes.LPVOID()
    assert advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW('D:P(A;;GA;;;' + sid + ')(A;;GA;;;SY)', 1, ctypes.byref(descriptor), None), 'Private credential channel ACL failed'
    attributes = SecurityAttributes(ctypes.sizeof(SecurityAttributes), descriptor, False)
    secret_path = args.output / ('credential-' + uuid.uuid4().hex + '.tmp')
    # Owner/SYSTEM only, shared reads for the SDK, OS-managed deletion on last close.
    secret_handle = kernel.CreateFileW(str(secret_path), 0xC0010000, 0x00000007, ctypes.byref(attributes), 1, 0x04000100, None)
    kernel.LocalFree(descriptor)
    assert secret_handle != wintypes.HANDLE(-1).value, 'Private temporary credential file creation failed'
    written = wintypes.DWORD()
    buffer = ctypes.create_string_buffer(key)
    try:
        assert kernel.WriteFile(secret_handle, buffer, len(key), ctypes.byref(written), None) and written.value == len(key), 'Private credential write failed'
    except Exception:
        kernel.CloseHandle(secret_handle)
        raise
    finally:
        ctypes.memset(buffer, 0, len(buffer))
    reference = 'configured-m3-account'
    config = {'bindingId': 'm3-vendor-live', 'bindingRevision': 1, 'configurationRef': 'm3-vendor-live-v1',
              'configuration': {'endpointBaseUrl': origin, 'credentialRef': reference},
              'trust': {'publishers': [], 'revokedPlugins': [], 'revokedVersions': [], 'revokedKeys': [], 'denylistedHosts': [], 'isFresh': False, 'supportedFeatures': []},
              'network': {'origins': [origin], 'allowPrivate': False, 'allowEmbeddedAuthorization': False, 'deniedHosts': [], 'deniedNetworks': []},
              'credentials': [{'reference': reference, 'secretFile': str(secret_path), 'publicId': None, 'customHeader': None, 'modes': ['bearer'], 'origins': [origin], 'ttlMs': 180000}],
              'media': [], 'mediaTtlMs': 180000, 'transport': {'handshakeTimeoutMs': 10000, 'writeTimeoutMs': 180000, 'shutdownTimeoutMs': 10000, 'cancelGraceMs': 100}, 'probe': None}
    config_path = args.output / 'host-config.json'
    config_path.write_text(json.dumps(config), encoding='utf-8')
    catalog = json.loads(json.loads(args.catalog_receipt.read_text(encoding='utf-8'))['dataJson'])
    operation = catalog['capabilityContracts'][0]['operations'][0]
    payload = {'schemaVersion': 2, 'modelId': 'MiniMax-M3', 'capabilityKey': 'text.generate', 'operationId': operation['id'], 'operationVersion': operation['version'], 'inputSchemaDigest': operation['inputSchema']['digest'],
               'input': {'messages': [{'role': 'user', 'content': '请用一句中文说明你已收到文本接口测试请求，不要调用工具。'}], 'parameters': {'thinking': False, 'maxTokens': 128}},
               'media': [], 'configuration': config['configuration'], 'uploads': []}
    request = {'protocolVersion': 2, 'operation': 'submit', 'profileId': 'm3-vendor-live', 'capabilitySlot': 'text', 'requestId': 'm3.' + uuid.uuid4().hex, 'budgetMs': 180000, 'payloadJson': json.dumps(payload, ensure_ascii=False), 'bindingId': 'm3-vendor-live', 'negotiatedVersion': 2, 'enabledFeatures': []}
    input_path = args.output / 'submit.input.json'
    input_path.write_text(json.dumps(request, ensure_ascii=False, indent=2), encoding='utf-8')
    workspace = args.output / 'workspace'
    output_path = workspace / 'receipts/submit.output.json'
    command = [str(args.cli), 'submit', '--sdk', str(args.sdk), '--worker', str(args.worker), '--package', str(args.package), '--workspace', str(workspace), '--config', str(config_path), '--input', str(input_path), '--output', str(output_path), '--create-workspace']
    try:
        run = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', timeout=200)
        secret_text = key.decode()
        (args.output / 'stdout.log').write_text(run.stdout.replace(secret_text, '[REDACTED]'), encoding='utf-8')
        (args.output / 'stderr.log').write_text(run.stderr.replace(secret_text, '[REDACTED]'), encoding='utf-8')
        response = json.loads(output_path.read_text(encoding='utf-8')) if output_path.exists() else None
        record = {'exitCode': run.returncode, 'credentialSource': str(args.app_data), 'credentialStoredInReceipt': False, 'credentialChannel': 'private current-user delete-on-close file', 'origin': origin, 'requestId': request['requestId'], 'packageSha256': hashlib.sha256((args.package / 'adapter.wasm').read_bytes()).hexdigest(), 'command': command, 'response': response}
        (args.output / 'result.json').write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')
        assert run.returncode == 0 and response and 'dataJson' in response, 'SDK vendor call failed; inspect the retained structured receipt, no retry attempted'
        result = json.loads(response['dataJson'])
        assert result['kind'] == 'completed' and len(result['outputs']) == 1 and result['outputs'][0]['kind'] == 'text'
        assert isinstance(result['outputs'][0]['text'], str) and result['outputs'][0]['text']
        assert result['data']['model'] == 'MiniMax-M3'
        kernel.CloseHandle(secret_handle)
        secret_handle = None
        assert not secret_path.exists(), 'Temporary credential was not removed'
        record['temporaryCredentialRemoved'] = True
        (args.output / 'result.json').write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')
        print(json.dumps({'status': 'PASS', 'vendorRequests': 1, 'origin': origin, 'text': result['outputs'][0]['text'], 'finishReason': result['data']['finishReason'], 'reportedUsage': result['usage'], 'receipt': str(args.output / 'result.json')}, ensure_ascii=False))
    finally:
        if secret_handle is not None:
            kernel.CloseHandle(secret_handle)
        receipt_path = args.output / 'result.json'
        if receipt_path.exists():
            record = json.loads(receipt_path.read_text(encoding='utf-8'))
            record['temporaryCredentialRemoved'] = not secret_path.exists()
            receipt_path.write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
