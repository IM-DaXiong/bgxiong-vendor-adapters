import hashlib,json,pathlib,subprocess,sys,shutil
SDK=pathlib.Path(sys.argv[1])
ROOT=pathlib.Path(sys.argv[2]);ROOT.mkdir(exist_ok=False)
CLI=pathlib.Path(sys.argv[3]);WORKER=pathlib.Path(sys.argv[4])
def save(path,value):path.write_text(json.dumps(value,indent=2),encoding='utf-8')
package=ROOT/'package';package.mkdir()
shutil.copyfile(SDK/'examples/cloud-task/target/wasm32-wasip2/release/bgx_example_cloud_task.wasm',package/'adapter.wasm')
manifest=json.loads((SDK/'examples/cloud-task/manifest.json').read_text());manifest['requiredFeatures'].append('image-projection-v1');manifest['artifactSha256']=hashlib.sha256((package/'adapter.wasm').read_bytes()).hexdigest()
save(package/'manifest.json',manifest)
(package/'manifest.sha256').write_text(''.join(hashlib.sha256((package/n).read_bytes()).hexdigest()+' '+n+'\n' for n in ['manifest.json','adapter.wasm']))
server=subprocess.Popen([sys.executable,str(SDK/'fixtures/services/async_api.py'),'--workspace',str(ROOT/'service'),'--media-host','localhost'],stdout=subprocess.PIPE,stderr=(ROOT/'service.stderr.log').open('w'),text=True)
try:
    ready=json.loads(server.stdout.readline());save(ROOT/'service.ready.json',ready);assert ready['event']=='ready',ready
    config={'bindingId': 'projection-binding', 'bindingRevision': 1, 'configurationRef': 'projection-config', 'configuration': {'endpointBaseUrl': 'http://127.0.0.1:1', 'credentialRef': None, 'scenario': 'normal', 'delayMs': 0, 'chunkDelayMs': 0, 'streamFormat': 'sse'}, 'trust': {'publishers': [], 'revokedPlugins': [], 'revokedVersions': [], 'revokedKeys': [], 'denylistedHosts': [], 'isFresh': False, 'supportedFeatures': ['stream-events-v1']}, 'network': {'origins': [], 'allowPrivate': True, 'allowEmbeddedAuthorization': False, 'deniedHosts': [], 'deniedNetworks': []}, 'credentials': [], 'media': [], 'mediaTtlMs': 60000, 'transport': {'handshakeTimeoutMs': 5000, 'writeTimeoutMs': 5000, 'shutdownTimeoutMs': 5000, 'cancelGraceMs': 100}, 'probe': None}
    config['network']['origins']=[ready['origin']]
    config['configuration']={'endpointBaseUrl':ready['origin'],'credentialRef':None,'scenario':'normal','queryScenario':'normal','uploadScenario':'normal','pollsUntilReady':2,'delayMs':0}
    config['trust']['supportedFeatures']=['image-projection-v1']
    save(ROOT/'config.json',config);save(ROOT/'initial-config.json',config);work=ROOT/'workspace'
    def command(args,name,expected=0):
        result=subprocess.run([str(CLI)]+args,capture_output=True,text=True,timeout=120)
        save(ROOT/f'{name}.command.json',{'argv':[str(CLI)]+args,'exitCode':result.returncode})
        (ROOT/f'{name}.stdout.json').write_text(result.stdout);(ROOT/f'{name}.stderr.log').write_text(result.stderr)
        assert result.returncode==expected,result.stdout+result.stderr
        return json.loads(result.stdout)
    def invoke(op,name,payload,create=False,root=None,expected=0):
        request={'protocolVersion':2,'operation':op,'profileId':config['bindingId'],'bindingId':config['bindingId'],'capabilitySlot':'image','requestId':name,'budgetMs':30000,'negotiatedVersion':2,'enabledFeatures':[],'payloadJson':json.dumps(payload)}
        save(ROOT/f'{name}.request.json',request)
        args=['caps' if op=='capabilities' else op,'--sdk',str(SDK),'--worker',str(WORKER),'--package',str(package),'--workspace',str(work),'--config',str(ROOT/'config.json'),'--input',str(ROOT/f'{name}.request.json'),'--output',str(work/'receipts'/f'{name}.json')]
        if create:args.append('--create-workspace')
        if root:args+=['--request-id',root]
        response=command(args,name,expected)
        return json.loads(response['dataJson']) if 'dataJson' in response else response
    caps=invoke('capabilities','caps',{},True)
    c=next(c for c in caps['capabilityContracts'] if c['capabilityKey']=='image.generate');op=c['operations'][0]
    root='panorama-original'
    accepted=invoke('submit',root,{'schemaVersion':2,'modelId':c['modelIds'][0],'capabilityKey':c['capabilityKey'],'operationId':op['id'],'operationVersion':op['version'],'inputSchemaDigest':op['inputSchema']['digest'],'input':{'prompt':'neutral asynchronous projection','strength':0.5,'outputProjection':'equirectangular360'},'media':[],'uploads':[],'configuration':config['configuration']})
    assert accepted['kind']=='accepted'
    task={'schemaVersion':2,'modelId':c['modelIds'][0],'capabilityKey':c['capabilityKey'],'operationId':op['id'],'operationVersion':op['version'],'vendorTaskId':accepted['vendorTaskId'],'resumeToken':accepted['resumeToken'],'configuration':config['configuration']}
    # Each call is a completed separate OS client, with the same frozen package.
    pending=invoke('query','query-pending',task,root=root)
    assert pending['status']=='running' and pending['data']=={'imageArtifacts':[]}
    denied=invoke('query','query-download-denied',task,root=root,expected=1)
    assert denied['error']['code']=='PERMISSION_DENIED'
    def inspect(name):
        return command(['retained','inspect','--package',str(package),'--workspace',str(work),'--config',str(ROOT/'config.json'),'--request-id',root,'--output',str(work/'receipts'/f'{name}.json')],name)
    retained=inspect('after-denied')
    assert retained['checkpointState']=='accepted'
    before=json.loads((ROOT/'service/state.json').read_text());save(ROOT/'before-retry.service.json',before)
    assert before['submitCount']==1
    config['network']['origins'].append(f"http://localhost:{ready['port']}");save(ROOT/'config-authorized.json',config)
    save(ROOT/'config.json',config)
    result=invoke('query','query-recovered',task,root=root)
    assert result['status']=='succeeded'
    retained=inspect('after-recovered')
    assert retained['checkpointState']=='terminal' and retained['checkpointResult']['data']==result['data']
    assert result['data']['imageArtifacts'][0]['mediaId']==retained['media'][0]['id']
    assert result['data']['imageArtifacts'][0]['projection']=='equirectangular360'
    state=json.loads((ROOT/'service/state.json').read_text());save(ROOT/'after-retry.service.json',state)
    assert state['submitCount']==1 and len(state['jobs'])==1
    assert state['jobs'][accepted['vendorTaskId']]['queryCount']==3
    assert result['usage'] is not None
    save(ROOT/'identities.json',{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [CLI,WORKER,package/'adapter.wasm',package/'manifest.json']})
    print('accepted -> fresh process pending -> denied download -> same ID recovered; submitCount=1; retained projection verified',flush=True)
finally:
    server.terminate();server.wait(timeout=10)
