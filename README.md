# Circus

Circus is a local coding agent. The process is a web server. You use it in a browser.

The stack is Rust, Axum, Askama, Hypergraft and Rig.

## Run

Install the tools.

```sh
mise install
pnpm install
```

Build the frontend assets.

```sh
mise run assets:build
```

Start the server.

```sh
mise run dev
```

Open `http://localhost:4000`.

Connect with an API key for one of these providers:

- xAI (Grok)
- OpenAI Codex
- Synthetic

The key stays in process memory. The key is not written to disk.

## Tasks

- `mise run dev` starts the development server.
- `mise run clean` formats the code and runs the checks.
- `mise run test` runs the test suite.
