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

They open that origin in a browser. They add a key for xAI, OpenAI Codex or Synthetic. Circus can store a key for each provider at the same time.

They choose the provider and model in chat. They can keep the provider default.

They send chat turns. Rig streams the reply. The transcript shows the reply as HTML.

Forget removes one provider key. The connect page stays available so they can add another provider.

Hypergraft updates page fragments. Ordinary links still work without it.

## Capabilities and Constraints

Current capabilities:

- Accept an API key for xAI, OpenAI Codex or Synthetic.
- Store more than one provider key on the local machine.
- Choose the provider and model in chat.
- Send chat turns through Rig.
- Stream the reply into the transcript as HTML.

Current constraints:

- Do not create user accounts.
- Do not call tools or edit files.
- Do not persist the transcript across process restarts.

Later work:

- Tools are intended soon. The first tools are undecided.
- A later agent can call any stored provider. The desk still has one transcript.

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
4. Circus stores provider keys on this machine until the user forgets that provider.
5. Do not claim work the product cannot do. Chat is current. Tools come later.
