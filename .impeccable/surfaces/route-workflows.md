---
version: 1
slug: "route-workflows"
primary_target: "route:/workflows"
related_targets:
    [
        "route:/workflows/new",
        "route:/workflows/{workflow_id}/configuration",
        "route:/conversations/{conversation_id}/workflow",
    ]
---

# Workflows

## Mode

Operate

## Scope

This surface covers the workflow catalogue, workflow authoring and the conversation launch sheet.

## Audience and job

A local developer chooses or edits a repeatable process before a model run starts.

## Hierarchy

The ordered process appears before configuration fields.

Run once and For each task appear before phase settings. A repeated group shows per-task phases once.

Each phase shows its purpose, candidate effect, model context boundary and approval route. Independent review is named only when that phase exists.

Configuration uses a phase selector. The selector does not submit the form. Hidden phase panels retain their values.

Environment overrides and technical mappings appear under Advanced settings.

Conversation setup starts with workflow names and short outcomes, not an expanded catalogue of phases.

The canonical conversation workflow GET uses explicit choose, inputs and review states. Navigation creates no run or execution reservation.

Only the selected process shows detailed phases through the shared process template. Its input state shows the required brief, plan or task list.

Model and preset overrides stay collapsed until requested. Review validates phase selections and reveals invalid controls. Error responses retain usable navigation controls.

The final review identifies selected context, effects, effective access, environment readiness and approval stops. Workers receive selected inputs, not the entire conversation history.

Workflows… opens setup from the conversation. Start workflow starts execution only after review. Back controls retain entered values.

Stale workflow selections return to the chooser without a substitute. Unavailable target selections retain their value and require an explicit replacement.

Storage integrity explanations stay outside the chooser.

Authoring retains ordinary phase controls. Advanced settings group environment overrides and technical mappings.

## Interaction

Use real buttons for phase selection. Keep the current phase visible and expose keyboard focus.

Keep all phase fields in the form. Hide inactive panels without disabling their inputs.

Show bounded revision destinations beside the review or approval phase that uses them.

Show invalid field values and server errors without replacement of submitted values.

If a hidden phase contains an invalid control, reveal that phase before browser validation moves focus.

If the server returns phase errors, reveal the first affected phase and open its advanced settings.

Keep the page usable at narrow widths. Let phase tabs scroll horizontally without changing the process order.

## Constraints

Use the existing case-file visual system, square corners and ruled sections.

Use DaisyUI controls and Tailwind layout utilities.

Use Australian English and sentence case for headings and buttons.

Do not introduce a new colour palette.
