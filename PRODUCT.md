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

The product stays local and account-free. The user brings a key or a plan login. A project desk turn starts a run against one project.

Success is a project file that an agent changes.

## Positioning

Power Plant is a thin local desk for coding agents. It is not an IDE. It is not a cloud workspace.

The user brings the provider key or plan login. Power Plant does not sell model access and does not create an account.

The user registers projects, agents, environments and workflows on this machine. Agents work in sandboxes on the local machine. The project desk stays a transcript plus composer.

## Operating Context

The user starts the Power Plant process on their machine. The default origin is `http://localhost:4000`.

They open that origin in a browser. They add a key or a plan login for a provider that is not stored. Power Plant can store one credential for each provider at the same time. The connect page offers only providers that are not stored.

They choose the provider, model and model-specific thinking effort on the project desk.

Projects are the first product object. Each project has a name and one immutable host path. The path must be a supported Git worktree. Users can create projects and rename them. They cannot edit paths or delete project records.

The product marks a project unavailable when its stored path no longer resolves. Unavailable records stay visible.

The `/` route uses the project catalogue. An empty catalogue redirects to `/projects/new`. One project redirects to `/projects/{project_id}`. Two or more projects redirect to `/projects`.

The project catalogue is not host access authority. An agent directory grant is the only authority for an agent to access a project. An agent is eligible when one stored canonical grant path equals the project path. Prefix matches give no authority. Project registration gives no authority.

They create agents in the local catalogue. Each agent has a name, instructions, tools and host directory grants. The selected project grant maps to `/project` during sandbox-backed steps. A workflow step can expose selected secondary grants as read-only context under `/access/<alias>`.

They create environment recipes with an OCI image and an optional setup script. A new installation includes a starter Git environment. Power Plant queues preparation for each recipe. A successful preparation creates a local snapshot. A workflow can use only a ready snapshot.

They create workflow definitions with a default environment, roles and ordered steps. A new installation includes starter workflow definitions. A step can run an agent, a registered system command or a human gate.

They open a project. If exactly one agent is eligible, the product opens that desk. If two or more agents are eligible, the product prefers the remembered eligible agent. If no remembered agent qualifies, the project page shows canonical desk links. If no agent is eligible, the page shows a starter-agent action. It shows grant actions for other current agents.

The canonical desk URL is `/projects/{project_id}/agents/{agent_id}`. The project path never appears in route parameters or query values. Agent configuration stays at `/agents` and `/agents/{agent_id}/configuration`.

Quick task is the default send mode on the project desk. Quick task is a system-owned one-agent run. It needs no configured workflow. It uses the pinned starter Git environment. The desk labels that environment Alpine Git. The product does not start a run when that prepared snapshot is absent or unavailable.

A read-only project grant produces a one-step Quick task. The step can answer and inspect files. It cannot produce a candidate revision.

A writable project grant produces three steps. An agent step creates a candidate. A human gate approves it. The current commit executor applies it.

An unchanged Quick task completes automatically after the assistant reply. The product creates no gate, decision artefact or commit attempt.

A changed Quick task waits at a human gate. The user must approve the exact candidate. The host worktree does not change before that approval. Approval creates a local Git commit through the current transaction path.

Configured workflows stay an advanced control on the same composer. The user can select a catalogue workflow. The control shows its review policy and environment preview.

Each run records project identity and run kind. Run kinds are Configured and Quick task. A conversation belongs to one project and agent pair.

The browser session remains the transcript owner. The session permits one active command. It can remember the last eligible agent and recent project order in memory only.

Each run pins the workflow definition and prepared environments. Run records track attempts, artefact references and human gates. Artefacts store candidate revisions and typed workflow outputs, such as plans, reviews, tests and human decisions.

A human-gate step pauses the run for a decision about an immutable candidate diff. For Quick task, the user can apply the candidate or discard the changes. For a configured workflow, the user can approve the candidate, request a revision or cancel the run. The run list shows the newest fifty runs.

Rig streams model replies on the host. An agent step can use the allowed list, read, write and run tools. Those tools run only in the guest. The transcript shows the reply as HTML. Tool traces appear in the transcript.

Forget removes one provider. The connect page stays available so they can add another provider.

Hypergraft updates page fragments. Ordinary links still work without it.

Project records, workflow definitions, agent records, environments, artefacts and run records persist locally. Browser transcripts remain memory-only.

## Capabilities and Constraints

Current capabilities:

- Accept an API key for xAI, OpenAI Codex, Synthetic, OpenRouter or DeepSeek.
- Sign in with a ChatGPT plan or a SuperGrok plan from the connect page.
- Store more than one provider on the local machine.
- Choose the provider, model and model-specific thinking effort on the project desk.
- Use bundled models.dev metadata when a model advertises adjustable thinking effort.
- Refresh the models.dev capability catalogue manually from Settings.
- Create projects in the local catalogue.
- Rename projects.
- Open a project desk with an eligible agent.
- Create a starter agent from a project.
- Grant a current agent access to a project.
- Create agents in the local catalogue.
- Edit and delete agents.
- Grant host directories to each agent.
- Configure the list, read, write and run tools for each agent.
- Create environment recipes from an OCI image and a setup script.
- Edit and delete environment recipes.
- Prepare environment snapshots for workflow use.
- Create workflow definitions.
- Edit and delete workflow definitions.
- Send a Quick task from the project desk with no workflow selection.
- Send a configured workflow from an advanced control on the same desk.
- Stream the reply into the transcript as HTML.
- Run sandbox-backed workflow steps in isolated guests.
- Expose the selected project at `/project` during sandbox-backed steps.
- Expose selected secondary directory grants as read-only context.
- List, read and write project files in the guest.
- Run a command in the guest.
- Complete an unchanged Quick task after the assistant reply.
- Show a candidate diff at a human gate.
- Apply an approved Quick task candidate as a local Git commit.
- Discard a Quick task candidate with no host change.
- Approve a candidate at a configured workflow gate.
- Request a revision at a configured workflow gate.
- Cancel a run at a human gate.
- Store run records, artefacts and human gate decisions on the local machine.
- Inspect run records and their artefacts.
- Show the newest fifty runs in the run list.
- Show tool traces in the transcript.
- Choose from five colour themes stored on the local machine.

Current constraints:

- The product does not create user accounts.
- The project desk stays a transcript plus composer.
- Agent tools run only in the guest. The guest does not receive a raw provider key or plan token. Microsandbox secrets inject placeholders. During agent steps, Power Plant limits outbound network access to the selected provider host. System-command steps have no network access. Model inference stays on the host through Rig.
- Project records, run records and artefacts persist locally. Browser transcripts remain memory-only. The product does not persist the transcript across process restarts.
- One desk job can be active per browser session.
- One workflow execution can be active process-wide.
- The project catalogue grants no host access. An agent directory grant remains the only authority.
- Project paths cannot change. Project records cannot be deleted in this release.
- Quick task needs the ready Alpine Git seed snapshot. The product does not fall back to another environment.
- Quick task never enters the workflow catalogue.

Later work:

- Task boards
- Parallel workflow execution

## Brand Commitments

The product name is Power Plant. The wordmark in the product is `Power Plant`. The mark is `app/public/images/logo.svg`.

UI copy uses Australian English. Capitalise only the first letter of a title, button or heading.

## Evidence on Hand

There are no testimonials, customers, benchmarks or launch claims. Do not invent them.

The product has connect, project, agent, workflow, environment and run surfaces. The logo file exists.

## Product Principles

1. The user runs Power Plant. There is no product account.
2. A hosted model is a connection the user brings, not a Power Plant service.
3. A project is the first product object. The desk stays thin. Power Plant is not an IDE and not a cloud workspace.
4. Power Plant stores a key or a plan login on this machine until the user forgets that provider.
5. Agents work in sandboxes. Success is a project file that an agent changes.
