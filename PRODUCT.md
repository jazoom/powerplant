# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

A developer wants a local agent that can change a project on their machine.

They already have a provider API key or a ChatGPT or SuperGrok plan. They want a local desk, not a cloud agent account.

## Product Purpose

Power Plant is a local coding agent. The user runs a web server on their machine. They talk to a hosted model in a browser. Agents work in sandboxes.

The Power Plant process stays on the host. It owns model calls, credentials, and sandbox lifecycle. Agent tools run only inside a guest virtual machine.

The product stays local and account-free. The user brings a key or a plan login. The chat surface stays thin.

Success is a project file that an agent changes.

## Positioning

Power Plant is a thin local desk for coding agents. It is not an IDE. It is not a cloud workspace.

The user brings the provider key or plan login. Power Plant does not sell model access and does not create an account.

Agents work in sandboxes on the local machine. The chat surface stays a transcript plus composer.

## Operating Context

The user starts the Power Plant process on their machine. The default origin is `http://localhost:4000`.

They open that origin in a browser. They add a key or a plan login for a provider that is not stored. Power Plant can store one credential for each provider at the same time. The connect page offers only providers that are not stored.

They choose the provider and model in chat. They can keep the provider default.

They choose a project directory. Power Plant bind-mounts that directory into a guest sandbox. Chat runs one built-in coding agent on that sandbox and project.

They send chat turns. Rig streams the model reply on the host. The agent can list, read and write files, run a command, and run git status, diff and commit. Those tools run only in the guest. The transcript shows the reply as HTML. Tool traces appear in the transcript.

Forget removes one provider. The connect page stays available so they can add another provider.

Hypergraft updates page fragments. Ordinary links still work without it.

Browser sessions hold the transcript in memory. Projects, sandboxes, and agent records belong on disk.

## Capabilities and Constraints

Current capabilities:

- Accept an API key for xAI, OpenAI Codex, Synthetic, OpenRouter or DeepSeek.
- Sign in with a ChatGPT plan or a SuperGrok plan from the connect page.
- Store more than one provider on the local machine.
- Choose the provider and model in chat.
- Send chat turns through Rig.
- Stream the reply into the transcript as HTML.
- Choose a project directory and bind-mount it into the sandbox.
- Run one built-in coding agent on the sandbox and project.
- List, read and write project files in the guest.
- Run a command in the guest.
- Run git status, diff and commit in the guest.
- Show tool traces in the transcript.

Current constraints:

- Do not create user accounts.
- The chat surface stays a transcript plus composer.
- Tools run only in the guest. The guest does not receive a raw provider key or plan token. Microsandbox secrets inject placeholders. Substitution and outbound network traffic are limited to the selected provider host. Model inference stays on the host through Rig.
- Do not persist the transcript across process restarts.
- One chat job runs at a time.

Later work:

- Several agents, task boards, and handoffs wait until one agent can change a repository.
- Custom images, user-supplied tools, persisted run history, and microsandbox cloud wait as well.

## Brand Commitments

The product name is Power Plant. The wordmark in the product is `Power Plant`. The mark is `app/public/images/logo.svg`.

UI copy uses Australian English. Capitalise only the first letter of a title, button or heading.

## Evidence on Hand

There are no testimonials, customers, benchmarks or launch claims. Do not invent them.

The running product has a connect surface and a chat surface. The logo file exists.

## Product Principles

1. The user runs Power Plant. There is no product account.
2. A hosted model is a connection the user brings, not a Power Plant service.
3. The desk stays thin. Power Plant is not an IDE and not a cloud workspace.
4. Power Plant stores a key or a plan login on this machine until the user forgets that provider.
5. Agents work in sandboxes. Success is a project file that an agent changes.
