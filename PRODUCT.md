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

The product stays local and account-free. The user brings a key or a plan login. A chat turn starts a workflow run against a project directory.

Success is a project file that an agent changes.

## Positioning

Power Plant is a thin local desk for coding agents. It is not an IDE. It is not a cloud workspace.

The user brings the provider key or plan login. Power Plant does not sell model access and does not create an account.

The user configures agents, environments and workflows on this machine. Agents work in sandboxes on the local machine. The chat surface stays a transcript plus composer.

## Operating Context

The user starts the Power Plant process on their machine. The default origin is `http://localhost:4000`.

They open that origin in a browser. They add a key or a plan login for a provider that is not stored. Power Plant can store one credential for each provider at the same time. The connect page offers only providers that are not stored.

They choose the provider and model in chat. They can keep the provider default.

They create agents in the local catalogue. Each agent has a name, instructions, tools and host directory grants. One grant is the primary project. Power Plant exposes it at `/project` during sandbox-backed steps. A workflow step can expose selected secondary grants as read-only context under `/access/<alias>`. The primary project directory must be a supported Git worktree.

They create environment recipes with an OCI image and an optional setup script. A new installation includes a starter Git environment. Power Plant queues preparation for each recipe. A successful preparation creates a local snapshot. A workflow can use only a ready snapshot.

They create workflow definitions with a default environment, roles and ordered steps. A new installation includes starter workflow definitions. A step can run an agent, a registered system command or a human gate.

They open an agent desk from the catalogue. They choose a workflow and send a chat turn. That turn starts a local run for the selected workflow and primary project.

Each run pins the workflow definition and prepared environments. Run records track attempts, artefact references and human gates. Artefacts store candidate revisions and typed workflow outputs, such as plans, reviews, tests and human decisions.

A human-gate step pauses the run for a decision about an immutable candidate diff. The user can approve the candidate, request a revision or cancel the run. The run list shows the newest fifty runs.

Rig streams model replies on the host. An agent step can use the allowed list, read, write and run tools. Those tools run only in the guest. The transcript shows the reply as HTML. Tool traces appear in the transcript.

Forget removes one provider. The connect page stays available so they can add another provider.

Hypergraft updates page fragments. Ordinary links still work without it.

Workflow definitions, agent records, environments, artefacts, and run records persist locally. Browser transcripts remain memory-only.

## Capabilities and Constraints

Current capabilities:

- Accept an API key for xAI, OpenAI Codex, Synthetic, OpenRouter or DeepSeek.
- Sign in with a ChatGPT plan or a SuperGrok plan from the connect page.
- Store more than one provider on the local machine.
- Choose the provider and model in chat.
- Create agents in the local catalogue.
- Edit and delete agents.
- Grant host directories to each agent.
- Configure the list, read, write and run tools for each agent.
- Create environment recipes from an OCI image and a setup script.
- Edit and delete environment recipes.
- Prepare environment snapshots for workflow use.
- Create workflow definitions.
- Edit and delete workflow definitions.
- Select a workflow on the chat desk.
- Send chat turns through Rig.
- Stream the reply into the transcript as HTML.
- Run sandbox-backed workflow steps in isolated guests.
- Expose the primary project at `/project` during sandbox-backed steps.
- Expose selected secondary directory grants as read-only context.
- List, read and write project files in the guest.
- Run a command in the guest.
- Store run records, artefacts and human gate decisions on the local machine.
- Inspect run records and their artefacts.
- Show the newest fifty runs in the run list.
- Show a candidate diff at a human gate.
- Approve a candidate at a human gate.
- Request a revision at a human gate.
- Cancel a run at a human gate.
- Show tool traces in the transcript.

Current constraints:

- The product does not create user accounts.
- The chat surface stays a transcript plus composer.
- Agent tools run only in the guest. The guest does not receive a raw provider key or plan token. Microsandbox secrets inject placeholders. During agent steps, Power Plant limits outbound network access to the selected provider host. System-command steps have no network access. Model inference stays on the host through Rig.
- Browser transcripts remain memory-only. The product does not persist the transcript across process restarts.
- One chat job can be active per browser session.
- One workflow execution can be active process-wide.

Later work:

- Task boards
- Parallel workflow execution

## Brand Commitments

The product name is Power Plant. The wordmark in the product is `Power Plant`. The mark is `app/public/images/logo.svg`.

UI copy uses Australian English. Capitalise only the first letter of a title, button or heading.

## Evidence on Hand

There are no testimonials, customers, benchmarks or launch claims. Do not invent them.

The product has connect, chat, agent, workflow, environment and run surfaces. The logo file exists.

## Product Principles

1. The user runs Power Plant. There is no product account.
2. A hosted model is a connection the user brings, not a Power Plant service.
3. The desk stays thin. Power Plant is not an IDE and not a cloud workspace.
4. Power Plant stores a key or a plan login on this machine until the user forgets that provider.
5. Agents work in sandboxes. Success is a project file that an agent changes.
