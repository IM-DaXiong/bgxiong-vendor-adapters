"""Validate public design only; never builds SDK or calls any provider."""
from pathlib import Path
import hashlib,json
from jsonschema import Draft202012Validator
from referencing import Registry,Resource
ROOT=Path(__file__).resolve().parents[1]
load=lambda p:json.loads((ROOT/p).read_text(encoding='utf-8'))
def require(value,message):
    if not value:raise AssertionError(message)
schemas=[json.loads(p.read_text(encoding='utf-8')) for p in sorted((ROOT/'schemas').glob('*.json'))]
registry=Registry()
for s in schemas:
    Draft202012Validator.check_schema(s)
    registry=registry.with_resource(s['$id'],Resource.from_contents(s))
def v(name):return Draft202012Validator(load('schemas/'+name),registry=registry)
caps=load('examples/capabilities.json')['capabilities']
coverage=load('examples/coverage-capabilities.json')['capabilities']
for cap in caps+coverage:
    v('capability-contract.schema.json').validate(cap)
    resources={r['id']:r for r in cap['schemas']}
    require(len(resources)==len(cap['schemas']),'duplicate schema id')
    for r in resources.values():
        Draft202012Validator.check_schema(json.loads(r['schemaJson']))
        require(hashlib.sha256(r['schemaJson'].encode()).hexdigest()==r['digest'],'schema digest mismatch')
    refs=[cap['configurationSchema']]
    for op in cap['operations']:
        refs += [op[k] for k in ['inputSchema','outputSchema','resumeSchema'] if op[k] is not None]
        for c in op['uiSchema']['controls']:
            schema=json.loads(resources[op['inputSchema']['id']]['schemaJson'])
            require(c['fieldPointer'].removeprefix('/') in schema['properties'],'UI pointer is not an input field')
    for ref in refs:require(ref['id'] in resources and ref['digest']==resources[ref['id']]['digest'],'unresolved ref')
fixtures=load('examples/payloads.json')
validator=v('operation-payload.schema.json')
require(validator.is_valid(fixtures['validSubmit']),'valid submit rejected')
require(not validator.is_valid(fixtures['invalidHostLeak']),'host data leaked across adapter boundary')
require(validator.is_valid(fixtures['invalidOperationInput']),'outer envelope hardcodes business fields')
for cap in caps:
    schema=json.loads(next(r['schemaJson'] for r in cap['schemas'] if r['id']=='input'))
    Draft202012Validator(schema).validate(fixtures['validSubmit']['input'])
    require(not Draft202012Validator(schema).is_valid(fixtures['invalidOperationInput']['input']),'operation input not strictly checked')
v('stream-event.schema.json').validate({'requestId':'r','sequence':1,'type':'text_delta','payload':{'channel':'answer','text':'x'}})
require(not v('stream-event.schema.json').is_valid({'requestId':'r','sequence':0,'type':'text_delta','payload':{'channel':'answer','text':'x'}}),'zero stream sequence accepted')
v('result.schema.json').validate({'kind':'accepted','vendorTaskId':'real-id-example','appliedParams':None,'resumeToken':None,'usage':None})
v('result.schema.json').validate({'status':'running','outputs':[],'data':None,'usage':None,'progress':None,'terminalFailure':None,'resumeToken':None})
v('result.schema.json').validate({'kind':'completed','vendorTaskId':None,'appliedParams':None,'outputs':[],'data':{'labels':['example']},'usage':None})
v('result.schema.json').validate({'status':'failed','outputs':[],'data':None,'usage':None,'progress':None,'terminalFailure':{'category':'vendorTaskFailed','message':'example','vendorCode':None,'details':None},'resumeToken':None})
bad={'status':'failed','outputs':[],'data':None,'usage':None,'progress':None,'terminalFailure':None,'resumeToken':None}
require(not v('result.schema.json').is_valid(bad),'terminal failure reason missing but accepted')
v('usage.schema.json').validate({'measurements':[{'metric':'output_tokens','unit':'tokens','amount':'12','source':'reported'}]})
# All dispatch refs resolve locally; no network or arbitrary fallback schemas.
for operation, pair in load('operation-contracts.json')['operations'].items():
    for targets in pair.values():
        for target in targets if isinstance(targets,list) else [targets]:
            name,_,pointer=target.partition('#')
            schema=load('schemas/'+name)
            for part in pointer.lstrip('/').split('/') if pointer else []:
                schema=schema[part.replace('~1','/').replace('~0','~')]
            Draft202012Validator.check_schema(schema)
control=load('schemas/control-operations.schema.json')['$defs']
Draft202012Validator(control['cancelResult']).validate({'outcome':'local_only'})
require(not Draft202012Validator(control['cancelResult']).is_valid({'outcome':'cancelled'}),'fake cancelled accepted')
Draft202012Validator(control['probeInput']).validate({'schemaVersion':2,'kind':'package_validation','configuration':{}})
v('capability-contract.schema.json').validate(coverage[1])
require(coverage[1]['transportSlot']=='generic','generic capability fixture missing')
require(coverage[2]['modelIds']==coverage[3]['modelIds'],'multi-capability fixture lost')
for schema in schemas:
    def refs(node):
        if isinstance(node,dict):
            if '$ref' in node:
                ref=node['$ref']
                if not ref.startswith('#'):require(any(ref.split('#')[0]==s['$id'] for s in schemas),'nonlocal public ref')
            for child in node.values():refs(child)
        elif isinstance(node,list):
            for child in node:refs(child)
    refs(schema)
print(f'DESIGN_CHECK=PASS schemas={len(schemas)} adapterShapes={len(caps)} coverageCapabilities={len(coverage)} contractRefs=PASS positiveAndNegativePayloads=PASS')
print('SDK_IMPLEMENTATION=NOT_STARTED RUNTIME_CONFORMANCE=NOT_RUN')
