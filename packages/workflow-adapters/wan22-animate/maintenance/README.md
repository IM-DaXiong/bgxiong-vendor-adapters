# Offline tracking completion export

This maintenance tool maps preserved evidence from an already completed Comfy tracking prompt. It is separate from runtime package contents. It never contacts Comfy, calls submit/query, edits a task, or changes an installed package. Python and Pillow are required.

```text
python export_tracking_completion.py --preserved <unique-preserved-evidence-directory> --out <new-export-directory>
python -m unittest discover -s . -p "test_*.py" -v
```

The preserved directory contains exact original checkpoint/frozen/payload, SDK submit invocation and before-send log, an authentic full history HTTP response, sealed manifest HTTP and disk copies, original uploaded source bytes, all mask PNGs, and `preserved-inventory.json` with absolute paths and SHA256. The tool checks the original client/request identity, full submitted versus executed graph, frozen endpoint/input/source/selection/direction/range/schema/resolved-frame bindings, successful execution, and indexed raw-mask pixel digest. Missing original POST acceptance responses must be disclosed; the tool does not reconstruct them. The maintenance authority reviews the authentic history and evidence chain.

`<requestId>.completion.json` contains exactly `{result,media}`. The completed result has the authentic vendor prompt ID and no resume token. Its outputs array must remain empty until the maintenance SDK actually imports every listed file through its MediaBroker with the specified media ID, then fills outputs using real leased-media metadata/handles. The SDK must validate the completed result against the original frozen contract and use exact revision CAS. Do not submit the empty-output template directly as a validated completion.

PNG descriptors use image/image/png. Frame-page and index JSON descriptors use opaque file/application/octet-stream, matching the public broker contract. Generated IDs are references for real imports, never fabricated handles. `evidence-files.json` includes the completion file itself, so the authorized host can bind the exact mapping bytes in its public reviewed evidence.

Exports never overwrite existing directories. Failures leave evidence untouched; any partial export must be retained and a new output directory selected. The four CPU tests use labeled synthetic fixtures; they are not remote task evidence or product acceptance.
