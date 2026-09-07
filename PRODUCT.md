# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

A developer wants a local agent that can change a project on their machine.

They already have a provider API key or a ChatGPT or SuperGrok plan. They want a local conversation workspace, not a cloud agent account.

## Product Purpose

Power Plant is a local coding agent. The user runs a web server on their machine. They talk to a hosted model in a browser. Agents work in sandboxes.

The Power Plant process stays on the host. It owns model calls, credentials, and sandbox lifecycle. Agent tools run only inside a guest virtual machine.

The product stays local and account-free. The user brings a key or a plan login.

Independent conversations are first-class work destinations. A conversation can discuss a general topic or receive explicit directory authority.

Success is a project file that an agent changes.

## Positioning

Power Plant is a thin local desk for coding agents. It is not an IDE. It is not a cloud workspace.

The user brings the provider key or plan login. Power Plant does not sell model access and does not create an account.

The user registers projects, agents, environments and workflows on this machine. Agents work in sandboxes on the local machine. Conversations retain their discussion history on this machine.

A conversation does not require a named agent or project. The user selects a model directly or applies an optional preset. Project attachments provide context references only until the user grants access.

A preset stores one complete snapshot of the implemented conversation settings. It includes the model, instructions, tools, environment, network policy and directory grants.

Preset application replaces the conversation settings after a full preview. It does not merge hidden permissions or copy sensitive-access consent.

A preset retains descriptive source provenance. Later source changes do not alter conversations or workflow phases that already copied its values.

Preset previews expire after 30 minutes. Each preview binds its replacement to the session and the destination settings revision or draft digest.

Preset application removes legacy project permissions. Missing providers, environments and directories remain requested values without substitutions or access approval.

Configured workflow phases reject preset access expansions and environment differences. Project-backed phases also reject presets that omit their required source directory.

A conversation can grant ad hoc Read only access to as many as eight host directories. A directory needs no project or Git registration.

Each grant stores the canonical directory identity and a stable `/access/<alias>` guest path. Duplicate and overlapping roots are invalid.

The first authorised root is the default command directory. Without a grant, tools use private scratch storage at `/workspace`.

Power Plant revalidates the device and inode before execution. A missing or replaced root remains visible as unavailable and does not retarget.

Sandbox network access defaults to Off. Conversation settings permit restricted domains or public internet access. Private and host networks remain excluded. Provider traffic remains separate and leaves the host for model inference.

A tool-enabled conversation message starts the system-owned Quick task. Without directory grants, tools use private scratch storage at `/workspace`. Power Plant mounts granted directories read only beside that scratch storage.

Scratch storage belongs to one attempt, not the conversation. Source-free tools create no Git candidate, code approval gate or commit.

A Review before apply message creates an isolated candidate. The user reviews the exact candidate before Power Plant updates ordinary host files.

Generic application creates no Git commit. Power Plant checks every directory, baseline, candidate object and pinned exclusion before the first write.

Each reviewed directory stays bound to its stable grant identity. The preview identifies changes by directory and relative path.

Application journals retain progress and outcomes for each directory. Writes across filesystems are not atomic.

A later conflict can leave an earlier directory applied. Recovery retains known per-directory outcomes and releases the execution blocker when no uncertainty remains. Partial application never appears as Completed.

Recovery restores only known transaction states. Unrelated host edits cause a conflict. Uncertain recovery retains evidence and blocks execution.

Reviewed capture excludes the application journals and other listed engine paths. Generic application never writes to those exclusions or Git administration.

## Operating Context

The user starts the Power Plant process on their machine. The default origin is `http://localhost:4000`.

They open that origin in a browser. They add a key or a plan login for a provider that is not stored. Power Plant can store one credential for each provider at the same time. The connect page offers only providers that are not stored.

They choose a provider, model and model-specific thinking effort in each conversation.

Conversations are the main work destination. Each project has a name and one immutable host path. The path must be a supported Git worktree. Users can create projects and rename them. They cannot edit paths or delete project records.

The product marks a project unavailable when its stored path no longer resolves. Unavailable records stay visible.

The `/` route opens `/conversations`. The conversations route lists local history and offers a New conversation action. A conversation can start without a project or preset.

The new-conversation page has no record or conversation identity. Navigation and invalid submissions create nothing in memory or local storage.

Only a valid first message creates the record and replaces the page URL with its canonical address. Model, effort, preset, directory and title choices remain unsaved page state before that message.

The folder chooser can add grants before that message. Chooser cancellation creates no grant or conversation record.

Provider choices contain connected providers only. Model and thinking effort dropdowns use the models.dev snapshot. A model without adjustable effort shows Not available.

Project-scoped entry carries an intended project reference, not file access. Saved conversations offer explicit project grants, documents, workflows and review work.

At the conversation limit, Power Plant rejects a new save. Existing conversations remain unchanged.

The project catalogue is not host access authority. Explicit conversation grants authorise conversation work. Saved-agent execution uses agent directory grants. An agent is eligible when one stored canonical grant path equals the project path. Prefix matches give no authority. Project registration gives no authority.

They create agents in the local catalogue. Each agent has a name, instructions, tools, a network policy and host directory grants. The selected project grant maps to `/project` during sandbox-backed steps. A workflow step can expose selected secondary grants as read-only context under `/access/<alias>`.

An agent network policy permits no network, selected domain suffixes or the public internet. No network is the default. Public access excludes the host and private networks. Restricted access includes each listed domain, its subdomains and all ports.

They create environment recipes with an OCI image and an optional setup script. A new installation includes a starter Git environment. Power Plant selects that starter for new conversations by default. The user can select another recipe for later attempts.

An idle conversation owns no persistent sandbox. An environment change selects the ready snapshot for subsequent attempts only.

An active task needs an explicit Stop task and switch action. Power Plant waits for managed cancellation and sandbox cleanup before it saves the selection.

Pending reviewed changes need review or an exact-candidate discard before a switch. A discard retains immutable evidence, conversation history and existing host files.

Power Plant queues preparation for each recipe. A successful preparation creates a local snapshot. Each tool attempt pins the selected ready snapshot. The conversation does not own a persistent sandbox. Tool-free chat requires no sandbox runtime or snapshot.

They create workflow definitions with a default environment, roles and ordered steps. A definition can run once or repeat one group of phases for each remaining task. A task list is not required for a one-shot definition. Ordinary chat still needs no workflow.

A new installation includes six starter processes.

The starters are:

- Plan a change
- Review current code
- Implement with approval
- Implement and review
- Plan then implement
- Ralph task loop

Each process preview shows its required inputs, candidate effects and approval stops. Independent review appears only when that process contains a separate read-only review phase. Ralph repeats implementation and optional review-and-fix. It does not add an extra independent review. A step can run an agent, a registered system command or a human gate.

They open a project to see its conversations. The project page lists conversations that reference the project and offers New conversation. The new conversation form carries the project reference without granting file access.

The old project-and-agent desk is not a conversation route. Project work starts at a conversation. Agent configuration stays at `/agents` and `/agents/{agent_id}/configuration`.

A writable conversation message starts the system-owned Quick task. Quick task is a system-owned one-agent run. It needs no configured workflow. It uses the environment selected in the conversation. The product does not start a run when that selected snapshot is absent or unavailable.

A read-only project grant produces a one-step Quick task. The step can answer and inspect files. It cannot produce a candidate revision.

A writable project grant produces three steps. An agent step creates a candidate. A human gate approves it. The current commit executor applies it.

An unchanged Quick task completes automatically after the assistant reply. The product creates no gate, decision artefact or commit attempt.

A changed Quick task waits at a human gate. The user must approve the exact candidate. The host worktree does not change before that approval. Approval creates a local Git commit through the current transaction path.

Configured workflows start from Run workflow beside the conversation title. The launch sheet shows its brief, target, access, environment readiness and fresh-context boundary.

Conversation documents include immutable task lists. Create tasks from this plan sends the selected plan to the conversation model without guest tools. Preparation never executes tasks.

A completed model response or submitted text can become a task list after validation. An explicit import reads one authorised project file through a read-only guest.

Task lists retain their preamble, checked tasks and indented details. Unchecked tasks are eligible. Fenced examples are not executable entries. Preview, linked review and Pi export use the saved revision.

The Ralph task loop starter runs each remaining task in order. It pins the task list, workflow body and phase selections at launch. Each task receives fresh implementation and optional review-and-fix contexts. The complete task file remains context, but each worker receives only one assigned task. Human approval precedes each task commit by default. An extra independent review is present only when the selected definition includes that phase.

One parent record shows task progress and links to child evidence. A successful commit or explicit no-change result permits the next task. The next task starts from the completed code, not an earlier worker transcript. A child approval gate retains the parent conversation reservation but releases execution reservations for another conversation. Earlier commits remain if a later task fails.

A workflow can declare a saved plan input. The authoring form offers Implementation from a saved plan without manual input keys or a task list.

The launch sheet selects an immutable plan revision from the conversation. The run retains its own copy and the source reference. Only phases that declare that input receive the plan. Later corrections or association removal cannot change the run copy. The run inspector exposes that copy.

Project-backed runs record the actual project identity. Source-free Quick tasks record neither a project identity nor an agent identity. Run kinds are Configured and Quick task. A run can belong to an independent conversation.

The browser session remains a command boundary, not a conversation identity. The session permits one active command while a conversation reservation protects its unfinished operation. A safe approval gate releases the session reservation so another conversation can run a command.

Each run pins the workflow definition and prepared environments. Run records track attempts, artefact references and human gates. Artefacts store candidate revisions and typed workflow outputs, such as plans, reviews, tests and human decisions.

A human-gate step pauses the run for a decision about an immutable candidate diff. Writable Quick tasks also support Request changes. Configured gates offer this action only with a declared revision route.

A human revision starts a fresh implementation attempt from the rejected candidate. It retains the original task brief and diff base, plus candidate-bound feedback. Earlier implementation transcripts stay outside the new context. Required reviews and code approval repeat before commit.

Revision limits apply across reopened gates. An exhausted limit blocks the run and retains its evidence. Model verdict routes remain distinct from human revision routes.

The launch sheet offers human approval before commit or automatic commit after an approved exact-candidate review when the definition supports that choice. Automatic commit requires review assurance. Read-only workflows need no commit policy or code approval.

The run list shows the newest fifty runs.

Rig streams model replies on the host. An agent step can use the allowed list, read, write and run tools. Those tools run only in the guest. The transcript shows the reply as HTML. Tool traces appear in the transcript.

Forget removes one provider. The connect page stays available so they can add another provider.

Hypergraft updates page fragments. Ordinary links still work without it.

Conversation history and project configuration persist locally. Browser sessions hold request reservations, not conversation identities.

Theme and thinking visibility preferences persist on the local machine.

## Capabilities and Constraints

Current capabilities:

- Accept an API key for xAI, OpenAI Codex, Synthetic, OpenRouter or DeepSeek.
- Sign in with a ChatGPT plan or a SuperGrok plan from the connect page.
- Store more than one provider on the local machine.
- Choose the provider, model and model-specific thinking effort in a conversation.
- Use bundled models.dev metadata when a model advertises adjustable thinking effort.
- Refresh the models.dev capability catalogue manually from Settings.
- Create projects in the local catalogue.
- Rename projects.
- Open a project and see its conversations.
- Create a conversation with a project reference.
- Grant conversation access to a project.
- Create agents in the local catalogue.
- Edit and delete agents.
- Grant host directories to each agent.
- Configure the list, read, write and run tools for each agent.
- Deny agent network access, allow selected domain suffixes or allow the public internet.
- Create environment recipes from an OCI image and a setup script.
- Edit and delete environment recipes.
- Prepare environment snapshots for workflow use.
- Select an environment for each conversation and its later tool attempts.
- Save conversation or draft settings as a named preset.
- Preview and apply a preset as a complete settings replacement.
- Stop active work or discard reviewed changes before an environment switch.
- Create workflow definitions, including Run once and For each task modes.
- Edit and delete workflow definitions.
- Send a Quick task from an independent conversation with no workflow selection.
- Use sandbox tools with private scratch storage and no project.
- Set sandbox network access to Off, restricted domains or public internet.
- Grant ad hoc Read only or Review before apply access to multiple directories without project registration.
- Preserve stable guest aliases and canonical host identity for conversation directories.
- Grant read-only or writable project authority from a conversation through the legacy project flow.
- Review a candidate diff and Apply or Discard it from the owning conversation.
- Review and apply one immutable candidate set across multiple ordinary directories without a Git commit.
- Recover interrupted file application from retained preimages and transaction progress.
- Launch a configured workflow from a conversation.
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
- Choose whether model replies show thinking details.
- Choose from five colour themes stored on the local machine.

Current constraints:

- The product does not create user accounts.
- A conversation stays a transcript plus composer with explicit project controls.
- Agent tools run only in the guest. The guest does not receive a provider key or plan token. Agent steps use the saved network policy. System-command steps have no network access. Environment preparation permits public destinations but excludes the host and private networks. Model inference stays on the host through Rig.
- Conversation history, presets, project records, run records and artefacts persist locally. The old project desk routes no longer exist.
- One session command can be active at a time.
- One unfinished operation can reserve a conversation.
- One workflow execution can be active process-wide.
- The project catalogue grants no file access. Conversation directory grants and saved-agent grants supply execution authority.
- Directory grants support Read only and multiple Review before apply roots. Sensitive access requires explicit consent.
- Tools cannot combine legacy project access with conversation directory grants.
- Project paths cannot change. Project records cannot be deleted in this release.
- Sandbox-backed Quick task needs the selected ready snapshot. The product does not fall back to another environment.
- Environment changes wait until active execution and pending reviewed work settle.
- Tool-free chat needs no sandbox runtime or prepared environment.
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
3. A conversation is the main work destination. Project access remains explicit. Power Plant is not an IDE or a cloud workspace.
4. Power Plant stores a key or a plan login on this machine until the user forgets that provider.
5. Agents work in sandboxes. Success is a project file that an agent changes.
