---
version: 1
slug: "route-projects"
primary_target: "route:/projects"
related_targets:
    [
        "route:/",
        "route:/projects/new",
        "route:/projects/folder",
        "route:/projects/{project_id}",
        "route:/projects/{project_id}/configuration",
        "route:/projects/{project_id}/agents/starter",
        "route:/projects/{project_id}/agents/{agent_id}",
        "route:/runs/{run_id}/gates/{gate_id}",
    ]
---

# Projects

## Mode

Operate

## Scope

This surface covers the project catalogue, project detail, the project desk and the Quick task gate.

## Audience and job

A local developer opens a project and sends work to one eligible agent.

## Operate-mode hierarchy

The product index lists Projects, Agents, Workflows, Environments and Runs.

The desk title is the project name. The host path sits under the title as quiet monospace metadata.

The selected agent is a compact control. Each eligible agent choice is a real canonical link.

The transcript sheet is the work record for this project and agent pair.

The composer dock is last. Sandbox status sits before the composer. Quick task Send is the primary action. Configured workflow is an advanced disclosure.

## Primary action

On the catalogue, the primary action is New project.

On an empty catalogue, the primary action is Create the first project.

On the new project page, the primary action is Choose project folder.

After a selected or manual path, the primary action is Add project.

On project detail with no eligible agent, the primary action is Create starter agent.

On project detail with two or more eligible agents and no remembered agent, the primary action is the canonical desk link for each agent.

On the project desk, the primary action is Send with Quick task.

On a Quick task gate, the primary action is Apply changes.

## Readiness states

The desk shows one sandbox status for the Alpine Git seed. The status sits before the composer.

An available ready snapshot has first priority, even when a replacement preparation is active or failed.

Status precedence:

1. Sandbox is ready: the ready snapshot is available. Quick task Send is enabled.
2. Sandbox preparation is in progress: no available ready snapshot, and the latest preparation is queued or active.
3. Sandbox preparation failed: no available ready snapshot, and the latest preparation failed or was interrupted.
4. Sandbox snapshot is invalid: no available ready snapshot, after active and failed states are excluded, and the ready snapshot is corrupt.
5. Sandbox is unavailable: no prior state applies.

Ready, failed, unavailable and invalid are terminal presentation states.

Failed, unavailable and invalid states link to the environment configuration route when the Alpine Git environment record exists. They fall back to `/environments` when the seed identifier or its current environment record is absent.

The composer stays disabled while the session owns an active command.

The composer stays disabled when the project path is unavailable.

The message field stays available while only the sandbox is not ready.

Quick task Send stays disabled while the sandbox is not ready.

Configured workflow send stays in the advanced disclosure. It uses that workflow environment preview.

## Empty states

No projects: the catalogue asks the user to create the first project. The new project page chooses an existing Git folder. Manual path entry remains available.

No agent: the project page states that no current agent has an exact directory grant. It explains that the agent can list, read, propose changes, and run sandbox commands. It states that host files remain unchanged until candidate approval. The primary command is Create starter agent. Configure permissions first remains the secondary route. It offers grant access when other agents exist.

Empty transcript: the desk shows Ask Power Plant. The lead is Write, explain or review code.

Unavailable project path: the desk shows a warning with a link to project configuration.

Sandbox not ready: the desk shows the sandbox status before the composer. Failed, unavailable and invalid states include the relevant environment route.

No configured workflows: the advanced disclosure links to create a workflow.

## Mobile topology

The mobile shell uses a compact masthead and a horizontal product row.

The mobile row labels are Projects, Agents, Workflows, Environments and Runs.

The Projects page is the project switcher.

The brand mark still goes to `/projects`.

The Working file label and the connection block are hidden.

The desk title row stacks. The composer dock stays at the end of the sheet.

## Flow

`/` reads the project catalogue.

An empty catalogue redirects to `/projects/new`.

One project redirects to `/projects/{project_id}`. Agent selection still occurs there.

Two or more projects redirect to `/projects`.

The catalogue orders projects by recent session use, then by stable catalogue order.

`/projects/new` presents the native folder chooser action. `POST /projects/folder` is a patch-only command.

The command opens a native dialog on the Power Plant host. It only accepts an existing Git project folder.

A selected folder fills the form with its canonical path. The folder command does not create or clone a project.

Cancellation returns the current form without an error.

A busy chooser returns a conflict patch and leaves the form intact.

`/projects/new?entry=manual` shows the Git project path field as the fallback.

The selected path stays visible and immutable before submission. The final `/projects` command validates and stores the project.

After a successful create, the product redirects to `/projects/{project_id}`.

The project page offers an explicit starter command when no eligible agent exists. `POST /projects/{project_id}/agents/starter` is a patch-only command. The command loads the current project record, then checks eligibility and creates at most one default agent in one catalogue operation.

The default agent uses the project name and empty instructions. It uses every built-in tool. The alias is `project`. Access is read-write. The sole directory grant is the stored project path. `primary_directory` is `project`.

If one eligible agent exists, the command opens that desk and does not create another record. If several eligible agents exist, the command returns to project detail and does not create another record. After a successful create, the command opens the new desk.

Configure permissions first remains a real link to `/agents/new?project={project_id}`.

The canonical desk URL is `/projects/{project_id}/agents/{agent_id}`.

Quick task needs no configured workflow. It uses the pinned Alpine Git environment.

An unchanged candidate completes after the assistant reply.

A changed candidate waits at the gate. The desk shows Review changes. That action links to the immutable candidate diff.

Quick task gate labels are Apply changes and Discard changes. The revision form is hidden.

A successful apply creates a local Git commit. Discard cancels the run and returns to the desk.

Configured workflow gates keep Approve candidate, Request revision and Cancel run.

## Constraints

Keep a real `href` on every ordinary navigation action.

Do not put the project path in route parameters or query values.

The folder command accepts patch representation only.

The starter command accepts patch representation only. It copies the stored project path. It does not take a host path from the request.

Do not put project names in the permanent mobile row.

Do not add a model-only chat path, a second executor or durable transcripts.

## Unresolved

None. This brief records the shipped flow.
