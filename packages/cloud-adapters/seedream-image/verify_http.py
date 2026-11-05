"""Local real HTTP through fixed public CLI/worker; never calls a vendor."""
import argparse
import base64
import copy
import hashlib
import io
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import random
import shutil
import socket
import subprocess
import threading
import uuid
from PIL import Image


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ('cli', 'sdk', 'worker', 'package', 'output'):
        parser.add_argument('--' + field, required=True, type=Path)
    parser.add_argument('--minimal', action='store_true')
    parser.add_argument('--extended-only', action='store_true', help='Reuse an unchanged successful minimal run; run only remaining cases')
    parser.add_argument('--cases', help='Comma-separated targeted cases; other valid results are not rerun')
    args = parser.parse_args()
    if any(not v.is_absolute() for v in vars(args).values() if isinstance(v, Path)):
        parser.error('All paths must be absolute')
    args.output.mkdir(parents=True, exist_ok=False)
    required_features = json.loads((args.package / 'manifest.json').read_text(encoding='utf-8-sig'))['requiredFeatures']
    fixtures = args.output / 'fixtures'
    fixtures.mkdir()
    rng = random.Random(20261025)
    image = Image.frombytes('RGB', (768, 768), rng.randbytes(768 * 768 * 3))
    paths = {}
    for name, fmt, img in [('large.png', 'PNG', image), ('small.jpg', 'JPEG', image.resize((64, 64))), ('small.webp', 'WEBP', image.resize((48, 48))), ('partial.png', 'PNG', image.resize((73, 79)))]:
        path = fixtures / name
        img.save(path, format=fmt)
        with Image.open(path) as decoded:
            decoded.load()
        paths[name] = path
    assert paths['large.png'].stat().st_size > 1048576
    binaries = {name: p.read_bytes() for name, p in paths.items()}
    scenario = {'name': 'large-url'}
    calls = []
    cdn_calls = []
    api_responses = {}
    original = '中文原文\n\n保留完整空行与 emoji 🐻'

    class Cdn(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            cdn_calls.append({'case': scenario['name'], 'path': self.path,
                              'authorizationPresent': self.headers.get('Authorization') is not None})
            name = self.path.removeprefix('/')
            status = 404 if name == 'missing' else 200
            value = binaries['large.png'] if name == 'truncated' else binaries.get(name, b'{"error":"missing"}')
            self.send_response(status)
            self.send_header('Content-Type', 'image/jpeg' if name.endswith('.jpg') else 'image/webp' if name.endswith('.webp') else 'image/png' if status == 200 else 'application/json')
            self.send_header('Content-Length', str(len(value)))
            self.end_headers()
            if name == 'truncated':
                self.wfile.write(value[:1024])
                self.wfile.flush()
                self.connection.shutdown(socket.SHUT_RDWR)
                self.connection.close()
                return
            self.wfile.write(value)

    cdn = ThreadingHTTPServer(('127.0.0.1', 0), Cdn)
    cdn_origin = 'http://127.0.0.1:' + str(cdn.server_port)

    class Api(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            raw = self.rfile.read(int(self.headers['Content-Length']))
            body = json.loads(raw)
            name = scenario['name']
            # Record real received bytes without ever treating a real key as fixture data.
            (args.output / (name + '.request-body.json')).write_bytes(raw)
            calls.append({'case': name, 'path': self.path, 'bodySha256': digest(raw),
                          'credentialMatched': self.headers.get('Authorization') == 'Bearer nonsecret-fixture',
                          'body': {k: v for k, v in body.items() if k != 'image'},
                          'referenceDataUrls': body.get('image')})
            data = [{'url': cdn_origin + '/large.png'}]
            if name == 'large-b64':
                data = [{'b64_json': base64.b64encode(binaries['large.png']).decode()}]
            elif name == 'aliases-and-duplicates':
                data = [{'image_url': cdn_origin + '/small.jpg'},
                        {'urls': [cdn_origin + '/small.webp', cdn_origin + '/small.jpg']},
                        {'b64': base64.b64encode(binaries['small.webp']).decode()},
                        {'image_base64': base64.b64encode(binaries['small.jpg']).decode()}]
            elif name == 'partial-output-failure':
                data = [{'url': cdn_origin + '/partial.png'}, {'url': cdn_origin + '/missing'}]
            elif name == 'partial-middle-failure':
                data = [{'urls': [cdn_origin + '/partial.png', cdn_origin + '/missing', cdn_origin + '/small.jpg']}]
            elif name == 'item-error-middle':
                data = [{'url': cdn_origin + '/partial.png'}, {'error': {'code': 'fixture-item'}}, {'url': cdn_origin + '/small.jpg'}]
            elif name == 'empty-item-first':
                data = [{}, {'url': cdn_origin + '/small.jpg'}]
            elif name == 'item-error':
                data = [{'url': cdn_origin + '/partial.png'}, {'error': {'code': 'fixture-item'}}]
            elif name == 'empty-item':
                data = [{'url': cdn_origin + '/partial.png'}, {}]
            elif name == 'empty-output':
                data = []
            elif name == 'not-an-image':
                data = [{'url': cdn_origin + '/not-image'}]
            elif name == 'download-disconnect':
                data = [{'url': cdn_origin + '/truncated'}]
            value = {'request_id': 'fixture-' + name, 'data': data,
                     'usage': {'input_tokens': 0, 'output_tokens': 12, 'total_tokens': 12,
                               'generated_images': len(data), 'other_reported_field': 123}}
            if name == 'bad-usage-valid-image':
                value['usage']['input_tokens'] = 'invalid-number'
            if name == 'bad-data':
                value['data'] = {'not': 'an array'}
            if name == 'native-image-count':
                value['usage']['image_count'] = 1
                value['usage'].pop('generated_images')
            status = {'unauthorized': 401, 'rate-limit': 429, 'insufficient-balance': 402}.get(name, 200)
            if status != 200:
                value = {'error': {'code': 'fixture-' + name}, 'request_id': 'fixture-' + name}
            if name == 'business-error':
                value = {'error': {'code': 'fixture-business'}}
            if name == 'no-usage':
                value.pop('usage')
            if name == 'post-disconnect':
                self.connection.shutdown(socket.SHUT_RDWR)
                self.connection.close()
                return
            wire = b'not-json' if name == 'malformed' else json.dumps(value).encode()
            api_responses[name] = wire
            (args.output / (name + '.response-body.json')).write_bytes(wire)
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(wire)))
            self.end_headers()
            self.wfile.write(wire)

    api = ThreadingHTTPServer(('127.0.0.1', 0), Api)
    api_origin = 'http://127.0.0.1:' + str(api.server_port)
    for server in (api, cdn):
        threading.Thread(target=server.serve_forever, daemon=True).start()
    secret = args.output / 'fixture-credential.txt'
    secret.write_text('nonsecret-fixture', encoding='utf8')
    config = {'bindingId': 'seedream-http-fixture', 'bindingRevision': 1, 'configurationRef': 'fixture-v1',
              'configuration': {'endpointBaseUrl': api_origin + '/api/v3', 'credentialRef': 'fixture-credential'},
              'trust': {'publishers': [], 'revokedPlugins': [], 'revokedVersions': [], 'revokedKeys': [], 'denylistedHosts': [], 'isFresh': False, 'supportedFeatures': required_features},
              'network': {'origins': [api_origin, cdn_origin], 'allowPrivate': True, 'allowEmbeddedAuthorization': False, 'deniedHosts': [], 'deniedNetworks': []},
              'credentials': [{'reference': 'fixture-credential', 'secretFile': str(secret), 'publicId': None, 'customHeader': None, 'modes': ['bearer'], 'origins': [api_origin], 'ttlMs': 180000}],
              'media': [], 'mediaTtlMs': 180000,
              'transport': {'handshakeTimeoutMs': 10000, 'writeTimeoutMs': 60000, 'shutdownTimeoutMs': 10000, 'cancelGraceMs': 100}, 'probe': None}
    config_file = args.output / 'host-config.json'
    workspace = args.output / 'workspace'
    records = []

    def invoke(operation, payload, label):
        config_file.write_text(json.dumps(config), encoding='utf8')
        request = {'protocolVersion': 2, 'operation': operation, 'profileId': config['bindingId'], 'capabilitySlot': 'generic' if operation == 'capabilities' else 'image', 'requestId': str(uuid.uuid4()), 'budgetMs': 120000, 'payloadJson': json.dumps(payload, ensure_ascii=False), 'bindingId': config['bindingId'], 'negotiatedVersion': 2, 'enabledFeatures': required_features}
        source = args.output / (label + '.input.json')
        target = workspace / 'receipts' / (label + '.output.json')
        source.write_text(json.dumps(request, ensure_ascii=False), encoding='utf8')
        command = [str(args.cli), 'caps' if operation == 'capabilities' else operation, '--sdk', str(args.sdk), '--worker', str(args.worker), '--package', str(args.package), '--workspace', str(workspace), '--config', str(config_file), '--input', str(source), '--output', str(target)]
        if not workspace.exists():
            command.append('--create-workspace')
        run = subprocess.run(command, capture_output=True, text=True, encoding='utf8', timeout=150)
        (args.output / (label + '.stdout.log')).write_text(run.stdout, encoding='utf8')
        (args.output / (label + '.stderr.log')).write_text(run.stderr, encoding='utf8')
        response = json.loads(target.read_text(encoding='utf8')) if target.exists() else None
        return run, response, command

    try:
        run, response, _ = invoke('capabilities', {}, 'capabilities')
        assert run.returncode == 0 and response and 'dataJson' in response, (run.returncode, run.stderr)
        op = json.loads(response['dataJson'])['capabilityContracts'][0]['operations'][0]
        # The public development host only imports sources inside its marked workspace.
        workspace_inputs = workspace / 'inputs'
        workspace_inputs.mkdir()
        for filename, path in paths.items():
            shutil.copyfile(path, workspace_inputs / filename)
        payload = {'schemaVersion': 2, 'modelId': 'doubao-seedream-5-0-pro-260628', 'capabilityKey': 'image.generate', 'operationId': op['id'], 'operationVersion': op['version'], 'inputSchemaDigest': op['inputSchema']['digest'], 'input': {'prompt': original, 'referenceImages': []}, 'media': [], 'uploads': [], 'configuration': config['configuration']}
        cases = [] if args.extended_only else [('large-url', None), ('large-b64', None)]
        if not args.minimal:
            cases += [('aliases-and-duplicates', None), ('single-reference', None), ('large-reference', None), ('ordered-references', None), ('ten-references', None), ('no-usage', None), ('native-image-count', None), ('unauthorized', 'UNAUTHORIZED'), ('rate-limit', 'RATE_LIMITED'), ('insufficient-balance', 'UPSTREAM_ERROR'), ('business-error', 'UPSTREAM_ERROR'), ('malformed', 'UPSTREAM_ERROR'), ('empty-output', 'UPSTREAM_ERROR'), ('not-an-image', 'UPSTREAM_ERROR'), ('unsupported-negative', 'INVALID_INPUT'), ('eleven-references', 'INVALID_INPUT'), ('denied-cdn', 'PERMISSION_DENIED'), ('partial-output-failure', 'UPSTREAM_ERROR')]
            cases += [('item-error', 'UPSTREAM_ERROR'), ('empty-item', 'UPSTREAM_ERROR'), ('post-disconnect', 'ACCEPTANCE_UNKNOWN')]
            cases += [('bad-usage-valid-image', 'UPSTREAM_ERROR'), ('bad-data', 'UPSTREAM_ERROR'), ('partial-middle-failure', 'UPSTREAM_ERROR'), ('item-error-middle', 'UPSTREAM_ERROR'), ('empty-item-first', 'UPSTREAM_ERROR')]
            cases += [('mismatched-reference-mime', 'PERMISSION_DENIED'), ('download-disconnect', 'UPSTREAM_ERROR')]
        if args.cases:
            requested = args.cases.split(',')
            assert set(requested) <= {name for name, _ in cases}
            cases = [(name, error) for name, error in cases if name in requested]
        for name, expected_error in cases:
            scenario['name'] = name
            p = copy.deepcopy(payload)
            config['media'] = []
            config['network']['origins'] = [api_origin] if name == 'denied-cdn' else [api_origin, cdn_origin]
            if name in ('single-reference', 'large-reference', 'ordered-references', 'ten-references', 'eleven-references', 'mismatched-reference-mime'):
                count = {'single-reference': 1, 'large-reference': 1, 'ordered-references': 2, 'ten-references': 10, 'eleven-references': 11, 'mismatched-reference-mime': 1}[name]
                for index in range(count):
                    filename = 'large.png' if name == 'large-reference' else ('small.jpg', 'small.webp')[index % 2]
                    media_id = 'reference-' + str(index)
                    raw = binaries[filename]
                    p['input']['referenceImages'].append({'mediaId': media_id})
                    p['media'].append({'id': media_id, 'kind': 'image', 'handle': 'provisioned', 'mime': 'image/png' if filename.endswith('.png') else 'image/jpeg' if filename.endswith('.jpg') else 'image/webp', 'fileName': digest(raw) + Path(filename).suffix, 'byteLength': str(len(raw))})
                    config['media'].append({'id': media_id, 'path': str(workspace_inputs / filename), 'kind': 'image'})
                p['input'].update(size='2720x1530', watermark=True, responseFormat='b64_json')
                if name == 'mismatched-reference-mime':
                    p['media'][0]['mime'] = 'image/png'
            if name == 'unsupported-negative':
                p['input']['negativePrompt'] = 'unsupported by current native contract'
            before, before_cdn = len(calls), len(cdn_calls)
            run, response, command = invoke('submit', p, name)
            actual_request = json.loads((args.output / (name + '.input.json')).read_text(encoding='utf8'))
            event_file = workspace / 'cli' / digest(actual_request['requestId'].encode()) / 'events.jsonl'
            events = [json.loads(line) for line in event_file.read_text(encoding='utf8').splitlines()] if event_file.exists() else []
            media_events = [e for e in events if e.get('type') == 'media_chunk']
            usage_events = [e for e in events if e.get('type') == 'usage']
            if required_features:
                assert [e['sequence'] for e in events] == list(range(1, len(events) + 1))
                assert all(e['requestId'] == actual_request['requestId'] for e in events)
                for event in media_events:
                    media = event['payload']['media']
                    assert (workspace / 'objects' / media['fileName']).stat().st_size == int(media['byteLength'])
            result = None
            if expected_error:
                error = (response or {}).get('error')
                if error is None:
                    error = json.loads(run.stderr.strip())
                assert error['code'] == expected_error, (name, run.returncode, error)
                if name in ('bad-usage-valid-image', 'bad-data', 'malformed', 'partial-middle-failure', 'item-error-middle', 'empty-item-first'):
                    raw_events = [e for e in media_events if e['payload']['role'] == 'provider-response']
                    assert len(raw_events) == 1
                    assert (workspace / 'objects' / raw_events[0]['payload']['media']['fileName']).read_bytes() == api_responses[name]
                    expected_images = {'bad-usage-valid-image': ['large.png'], 'bad-data': [], 'malformed': [],
                                       'partial-middle-failure': ['partial.png', 'small.jpg'],
                                       'item-error-middle': ['partial.png', 'small.jpg'], 'empty-item-first': ['small.jpg']}[name]
                    artifacts = [e for e in media_events if e['payload']['role'] == 'artifact']
                    assert len(artifacts) == len(expected_images)
                    for event, filename in zip(artifacts, expected_images):
                        assert (workspace / 'objects' / event['payload']['media']['fileName']).read_bytes() == binaries[filename]
                    assert len(usage_events) == (0 if name in ('bad-usage-valid-image', 'malformed') else 1)
                    if name != 'malformed':
                        assert error['vendorRequestId'] == 'fixture-' + name
                    if name == 'partial-middle-failure':
                        assert len(cdn_calls) - before_cdn == 3
                    if name == 'item-error-middle':
                        assert error['vendorCode'] == 'fixture-item'
                if name in ('partial-output-failure', 'item-error', 'empty-item'):
                    assert error['vendorRequestId'] == 'fixture-' + name
                    assert len([e for e in media_events if e['payload']['role'] == 'artifact']) == 1
                    assert len(usage_events) == 1
                if name == 'business-error':
                    assert error['vendorCode'] == 'fixture-business'
                if name == 'item-error':
                    assert error['vendorCode'] == 'fixture-item'
                assert len(calls) - before == (0 if name in ('unsupported-negative', 'eleven-references', 'mismatched-reference-mime') else 1)
                if name == 'download-disconnect':
                    assert error['vendorRequestId'] == 'fixture-' + name
                    assert not [e for e in media_events if e['payload']['role'] == 'artifact']
                    assert len(usage_events) == 1
                if name == 'denied-cdn':
                    assert len(cdn_calls) == before_cdn
                if name == 'partial-output-failure':
                    partial_sha = digest(binaries['partial.png'])
                    retained = list((workspace / 'objects').glob(partial_sha + '.*'))
                    assert retained and retained[0].read_bytes() == binaries['partial.png']
                    # Explicitly record the contract limitation: raw object survives,
                    # but failed Response has no usable structured output references.
                    artifact_events = [e for e in media_events if e['payload']['role'] == 'artifact']
                    if required_features:
                        assert len(artifact_events) == 1 and len(usage_events) == 1
                        assert artifact_events[0]['payload']['media']['fileName'] == retained[0].name
                        assert (workspace / 'objects' / artifact_events[0]['payload']['media']['fileName']).read_bytes() == binaries['partial.png']
                    (args.output / 'partial-boundary.json').write_text(json.dumps({'error': error, 'rawObjectRetained': str(retained[0]), 'sha256': partial_sha, 'structuredOutputsReturned': False, 'standardMediaEvents': len(artifact_events), 'reportedUsageEvents': len(usage_events), 'eventJournal': str(event_file), 'sdkChanged': False}), encoding='utf8')
            else:
                assert run.returncode == 0 and response and 'dataJson' in response, (name, run.returncode, run.stderr)
                result = json.loads(response['dataJson'])
                output_media = [o['media'] for o in result['outputs'] if o['role'] == 'artifact']
                expected = ['small.jpg', 'small.webp', 'small.jpg', 'small.webp', 'small.jpg'] if name == 'aliases-and-duplicates' else ['large.png']
                assert len(output_media) == len(expected)
                for media, filename in zip(output_media, expected):
                    actual = (workspace / 'objects' / media['fileName']).read_bytes()
                    assert actual == binaries[filename] and digest(actual) == digest(binaries[filename])
                    assert Path(media['fileName']).suffix == Path(filename).suffix
                raw_output = next(o['media'] for o in result['outputs'] if o['role'] == 'provider-response')
                assert (workspace / 'objects' / raw_output['fileName']).read_bytes() == api_responses[name]
                assert result['data']['imageCount'] == len(expected)
                if name == 'no-usage':
                    assert result['usage'] is None and result['data']['reportedUsageJson'] is None
                else:
                    assert result['usage']['measurements'][0]['amount'] == '0'
                    assert json.loads(result['data']['reportedUsageJson'])['other_reported_field'] == 123
                    if name == 'native-image-count':
                        measure = next(m for m in result['usage']['measurements'] if m['metric'] == 'local.bgx.seedream-image/image_count')
                        assert measure['amount'] == '1'
                if required_features:
                    assert len(media_events) == len(output_media) + 1
                    assert len(usage_events) == (0 if name == 'no-usage' else 1)
                    assert {e['payload']['media']['id'] for e in media_events} == {o['media']['id'] for o in result['outputs']}
                assert len(calls) - before == 1
            if len(calls) > before:
                call = calls[-1]
                assert call['path'] == '/api/v3/images/generations' and call['credentialMatched']
                assert call['body']['model'] == payload['modelId'] and call['body']['prompt'] == original
                assert call['body']['stream'] is False
                if name in ('single-reference', 'large-reference', 'ordered-references', 'ten-references'):
                    image_values = call['referenceDataUrls'] if isinstance(call['referenceDataUrls'], list) else [call['referenceDataUrls']]
                    assert len(image_values) == len(p['input']['referenceImages'])
                    for index, value in enumerate(image_values):
                        mime, encoded = value.split(';base64,', 1)
                        filename = 'large.png' if name == 'large-reference' else ('small.jpg', 'small.webp')[index % 2]
                        assert base64.b64decode(encoded) == binaries[filename]
                        assert mime == ('data:image/png' if filename.endswith('.png') else 'data:image/jpeg' if filename.endswith('.jpg') else 'data:image/webp')
                    assert call['body']['size'] == '2720x1530' and call['body']['watermark'] is True and call['body']['response_format'] == 'b64_json'
                else:
                    assert call['referenceDataUrls'] is None
                    assert call['body']['size'] == '2K' and call['body']['watermark'] is False and call['body']['response_format'] == 'url'
            assert all(not c['authorizationPresent'] for c in cdn_calls)
            records.append({'case': name, 'exitCode': run.returncode, 'expectedError': expected_error, 'httpCalls': len(calls) - before, 'cdnCalls': len(cdn_calls) - before_cdn, 'imageOutputs': None if result is None else result['data']['imageCount'], 'mediaEvents': len(media_events), 'usageEvents': len(usage_events), 'command': command})
            (args.output / 'cases.json').write_text(json.dumps(records, ensure_ascii=False, indent=2), encoding='utf8')
            (args.output / 'http-calls.json').write_text(json.dumps(calls, ensure_ascii=False, indent=2), encoding='utf8')
            (args.output / 'cdn-calls.json').write_text(json.dumps(cdn_calls, indent=2), encoding='utf8')
            print(json.dumps({'case': name, 'exitCode': run.returncode, 'expectedError': expected_error}), flush=True)
        print(json.dumps({'status': 'LOCAL_HTTP_COMPLETE', 'cases': len(records), 'largeImageBytes': len(binaries['large.png']), 'requiredFeatures': required_features, 'partialOutputRecovery': 'see partial-boundary.json if executed; final error and standard events are separate facts', 'vendorCalls': 0}), flush=True)
    finally:
        for server in (api, cdn):
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    main()
