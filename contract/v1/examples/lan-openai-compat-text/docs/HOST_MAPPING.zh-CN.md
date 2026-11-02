# HOST_MAPPING · LAN OpenAI-compatible text

This crate talks **OpenAI HTTP shape**, not Comfy `/prompt`.

| Credential archive | Origin | Notes |
|---|---|---|
| Third-party engine | user `endpointBaseUrl` | e.g. any `/v1/chat/completions` LAN port |
| This product llama | `http://<peer>:18380` | Wave B gateway. **Never** 18080–18089 |

| HOST_PAYLOAD | HTTP |
|---|---|
| `endpointBaseUrl` | origin only (no `/v1`) |
| `model` | JSON `model`; must be `example-lan-llm` or a `/v1/models` id |
| `messages` | POST body `messages` |
| `temperature` / `maxTokens` | `temperature` / `max_tokens` |
| `responseFormatJson` | `response_format: {type:json_object}` |
| `thinking` | default **rejected** |

Probe: `GET {origin}/v1/models` only when origin is set.  
Submit: same GET then `POST {origin}/v1/chat/completions`.  
Result: `choices[0].message.content` → `outputs[0].text`.
