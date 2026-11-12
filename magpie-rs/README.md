# magpie-rs

Rust-based AI gateway that embeds the CodeArts gateway and provides a unified
OpenAI-compatible chat completions endpoint routing to multiple providers.

## Routes

| Route | Description |
|---|---|
| `/healthz` | Health check |
| `/codearts/*` | Embedded CodeArts gateway |
| `/gemini/v1/chat/completions` | Gemini Provider (API Key) |
| `/claude/v1/chat/completions` | Claude Provider (API Key) |
| `/openai/v1/chat/completions` | OpenAI/Codex Provider (API Key) |
| `/v1/chat/completions` | Unified model router (auto-detects provider by model prefix) |

## Unified model routing

`POST /v1/chat/completions` automatically routes based on the `model` field:

| Model prefix | Provider |
|---|---|
| `gemini-*` | Gemini |
| `claude-*` | Claude |
| `gpt-*`, `o1-*`, `codex-*` | OpenAI |
| other | 400 Bad Request |

## Configuration

Copy `config.toml.example` to `config.toml` and fill in your values, or use
environment variables with the `MAGPIE_RS_` prefix:

```bash
export MAGPIE_RS_LISTEN=0.0.0.0:8080
export MAGPIE_RS_CODEARTS_AK=your_ak
export MAGPIE_RS_CODEARTS_SK=your_sk
export MAGPIE_RS_GATEWAY_KEY=your_gateway_key
export MAGPIE_RS_GEMINI_API_KEY=your_gemini_key
export MAGPIE_RS_CLAUDE_API_KEY=your_claude_key
export MAGPIE_RS_OPENAI_API_KEY=your_openai_key
```

## Build & run

```bash
cargo build --release
./target/release/magpie-rs
```

## Test

```bash
cargo test --all
```

## Configuration priority

Configuration is resolved in this order (later overrides earlier):

1. `config.toml` file (if present)
2. Environment variables with `MAGPIE_RS_` prefix

Example: `MAGPIE_RS_GEMINI_API_KEY` overrides `gemini_api_key` in `config.toml`.

## Example request

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-2.5-flash",
    "messages": [{"role": "user", "content": "hello"}],
    "stream": true
  }'
```

The response is an OpenAI-compatible SSE stream:

```
data: {"id":"chatcmpl-...","object":"chat.completion.chunk","model":"gemini-2.5-flash","choices":[{"index":0,"delta":{"content":"..."}}]}

data: [DONE]
```

## Provider-specific endpoints

If you prefer to hit a specific provider directly (bypassing unified routing):

```bash
# Gemini
curl -X POST http://localhost:8080/gemini/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"gemini-2.5-flash","messages":[{"role":"user","content":"hi"}],"stream":true}'

# Claude
curl -X POST http://localhost:8080/claude/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"hi"}],"stream":true}'

# OpenAI
curl -X POST http://localhost:8080/openai/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"stream":true}'
```
