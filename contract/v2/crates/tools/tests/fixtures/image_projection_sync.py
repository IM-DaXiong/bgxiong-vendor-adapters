import hashlib,json,pathlib,subprocess,sys,shutil
SDK=pathlib.Path(sys.argv[1])
ROOT=pathlib.Path(sys.argv[2]);ROOT.mkdir(exist_ok=False)
CLI=pathlib.Path(sys.argv[3]);WORKER=pathlib.Path(sys.argv[4])
def save(path,value):path.write_text(json.dumps(value,indent=2),encoding='utf-8')
package=ROOT/'package';package.mkdir()
shutil.copyfile(SDK/'examples/sync-api/target/wasm32-wasip2/release/bgx_example_sync_api.wasm',package/'adapter.wasm')
manifest=json.loads((SDK/'examples/sync-api/manifest.json').read_text());manifest['requiredFeatures'].append('image-projection-v1');manifest['artifactSha256']=hashlib.sha256((package/'adapter.wasm').read_bytes()).hexdigest()
save(package/'manifest.json',manifest)
(package/'manifest.sha256').write_text(''.join(hashlib.sha256((package/n).read_bytes()).hexdigest()+' '+n+'\n' for n in ['manifest.json','adapter.wasm']))
server=subprocess.Popen([sys.executable,str(SDK/'fixtures/services/sync_api.py'),'--workspace',str(ROOT/'service')],stdout=subprocess.PIPE,stderr=(ROOT/'service.stderr.log').open('w'),text=True)
try:
    ready=json.loads(server.stdout.readline());save(ROOT/'service.ready.json',ready)
    assert ready['event']=='ready',ready
    config={'bindingId': 'projection-binding', 'bindingRevision': 1, 'configurationRef': 'projection-config', 'configuration': {'endpointBaseUrl': 'http://127.0.0.1:1', 'credentialRef': None, 'scenario': 'normal', 'delayMs': 0, 'chunkDelayMs': 0, 'streamFormat': 'sse'}, 'trust': {'publishers': [], 'revokedPlugins': [], 'revokedVersions': [], 'revokedKeys': [], 'denylistedHosts': [], 'isFresh': False, 'supportedFeatures': ['stream-events-v1']}, 'network': {'origins': [], 'allowPrivate': True, 'allowEmbeddedAuthorization': False, 'deniedHosts': [], 'deniedNetworks': []}, 'credentials': [], 'media': [], 'mediaTtlMs': 60000, 'transport': {'handshakeTimeoutMs': 5000, 'writeTimeoutMs': 5000, 'shutdownTimeoutMs': 5000, 'cancelGraceMs': 100}, 'probe': None}
    config['network']['origins']=[ready['origin']];config['configuration']['endpointBaseUrl']=ready['origin'];config['trust']['supportedFeatures'].append('image-projection-v1')
    save(ROOT/'config.json',config);save(ROOT/'initial-config.json',config);work=ROOT/'workspace'
    def command(args,name,expected=0):
        result=subprocess.run([str(CLI)]+args,capture_output=True,text=True,timeout=120)
        save(ROOT/f'{name}.command.json',{'argv':[str(CLI)]+args,'exitCode':result.returncode})
        (ROOT/f'{name}.stdout.json').write_text(result.stdout);(ROOT/f'{name}.stderr.log').write_text(result.stderr)
        assert result.returncode==expected,result.stdout+result.stderr
        return json.loads(result.stdout)
    def invoke(op,name,payload,create=False):
        catalog=json.loads((SDK/'protocol/host-feature-catalog.json').read_text())
        enabled=[f['id'] for f in catalog['features'] if f['requiredImports'] and f['id'] in config['trust']['supportedFeatures']]
        request={'protocolVersion':2,'operation':op,'profileId':config['bindingId'],'bindingId':config['bindingId'],'capabilitySlot':'image','requestId':name,'budgetMs':30000,'negotiatedVersion':2,'enabledFeatures':enabled,'payloadJson':json.dumps(payload)}
        save(ROOT/f'{name}.request.json',request)
        args=['caps' if op=='capabilities' else op,'--sdk',str(SDK),'--worker',str(WORKER),'--package',str(package),'--workspace',str(work),'--config',str(ROOT/'config.json'),'--input',str(ROOT/f'{name}.request.json'),'--output',str(work/'receipts'/f'{name}.json')]
        if create:args.append('--create-workspace')
        return json.loads(command(args,name)['dataJson'])
    caps=invoke('capabilities','caps',{},True)
    c=next(c for c in caps['capabilityContracts'] if c['capabilityKey']=='image.generate');op=c['operations'][0]
    for projection in ['flat','equirectangular360']:
        result=invoke('submit',projection,{'schemaVersion':2,'modelId':c['modelIds'][0],'capabilityKey':c['capabilityKey'],'operationId':op['id'],'operationVersion':op['version'],'inputSchemaDigest':op['inputSchema']['digest'],'input':{'prompt':'neutral image projection fixture','strength':0.5,'outputProjection':projection},'media':[],'uploads':[],'configuration':config['configuration']})
        assert result['kind']=='completed'
        assert len(result['outputs'])==2
        assert len({o['media']['id'] for o in result['outputs']})==2
        assert any(m['metric'].endswith('/reported-cost') and m['unit'].endswith('/credits') for m in result['usage']['measurements'])
        args=['retained','inspect','--package',str(package),'--workspace',str(work),'--config',str(ROOT/'config.json'),'--request-id',projection,'--output',str(work/'receipts'/f'{projection}.inspect.json')]
        retained=command(args,projection+'.inspect')
        assert retained['checkpointResult']['data']==result['data']
        assert len(retained['media'])==2 and len({m['sha256'] for m in retained['media']})==1
        assert retained['media'][0]['id']==result['data']['imageArtifacts'][0]['mediaId']
        assert result['data']['imageArtifacts'][0]['projection']==projection
        selection=ROOT/f'{projection}.selection.json';save(selection,[m['id'] for m in retained['media']])
        exported=command(['retained','export','--package',str(package),'--workspace',str(work),'--config',str(ROOT/'config.json'),'--request-id',projection,'--output',str(work/'receipts'/f'{projection}.export.json'),'--selection',str(selection),'--destination',str(ROOT/f'{projection}-export')],projection+'.export')
        raw=pathlib.Path(exported['destination'])/exported['media'][0]['relativePath']
        assert hashlib.sha256(raw.read_bytes()).hexdigest()==retained['media'][0]['sha256']
        print(projection,'profile negotiation, CLI, checkpoint, retained export PASS',flush=True)
    save(ROOT/'identities.json',{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [CLI,WORKER,package/'adapter.wasm',package/'manifest.json']})
finally:
    server.terminate();server.wait(timeout=10)
