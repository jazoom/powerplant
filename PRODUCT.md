# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

A developer wants a local agent that can call hosted models.

They work on their own machine. They already have a provider API key. They want a local desk, not a cloud agent account.

## Product Purpose

Circus is a local coding agent. The user runs a web server on their machine. They talk to a hosted model in a browser.

Success is a connected session that streams a useful reply into the transcript.

The longer job is a coding agent that can use tools. Tools are intended soon. The first tools are undecided.

## Positioning

Circus is a thin local desk for a coding agent. It is not an IDE. It is not a cloud workspace.

The user brings the provider key. Circus does not sell model access and does not create an account.

## Operating Context

The user starts the Circus process on their machine. The default origin is `http://localhost:4000`.

They open that origin in a browser. They choose xAI, OpenAI Codex or Synthetic. They paste an API key. They can set a model name or keep the provider default.

They send chat turns. Rig streams the reply. The transcript shows the reply as HTML.

Disconnect ends the session and drops the key from memory.

Hypergraft updates page fragments. Ordinary links still work without it.

## Capabilities and Constraints

Current capabilities:

- Accept an API key for xAI, OpenAI Codex or Synthetic.
- Hold the key in process memory for the browser session.
- Send chat turns through Rig.
- Stream the reply into the transcript as HTML.

Current constraints:

- Do not create user accounts.
- Do not write API keys to disk.
- Do not call tools or edit files.

Later work:

- Tools are intended soon. The first tools are undecided.
- Circus will offer reuse of tokens and keys across sessions. The mechanism is undecided.

## Brand Commitments

The product name is Circus. The wordmark in the product is `circus`. The mark is `app/public/images/logo.svg`.

UI copy uses Australian English. Capitalise only the first word of a title, button or heading.

## Evidence on Hand

There are no testimonials, customers, benchmarks or launch claims. Do not invent them.

The running product has a connect surface and a chat surface. The logo file exists.

## Product Principles

1. The user runs Circus. There is no product account.
2. A hosted model is a connection the user brings, not a Circus service.
3. The desk stays thin. Circus is not an IDE and not a cloud workspace.
4. Secrets stay in process memory until a reuse mechanism exists.
5. Do not claim work the product cannot do. Chat is current. Tools come later.
