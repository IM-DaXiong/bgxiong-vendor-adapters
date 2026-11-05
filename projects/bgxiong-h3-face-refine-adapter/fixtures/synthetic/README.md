# Synthetic fixtures

Distributable placeholders only. Do **not** commit the user's private 107-frame video.

For A05 live acceptance on the developer machine, set env:

```
H3_FACE_SOURCE_VIDEO=<absolute path to local 107-frame mp4>
H3_FACE_APPEARANCE_REF=<absolute path to appearance still>
H3_FACE_IDENTITY_REF=<absolute path to identity still>
H3_FACE_COMFY_BASE=http://127.0.0.1:8189
H3_FACE_OUT_DIR=<writable output directory>
```

Expected selection for the known case: distant woman (not near man). Harness never uploads user media off-machine.
