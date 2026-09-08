---
name: Power Plant
description: A local conversation workspace that presents project work as an indexed repository case file.
colors:
    cover: "oklch(26.7% 0.0333 118.41)"
    coverDeep: "oklch(19.34% 0.0197 121.82)"
    paper: "oklch(95.96% 0.0434 100.07)"
    paperGreen: "oklch(93.29% 0.0576 119.18)"
    sky: "oklch(83.6% 0.0646 212.34)"
    ink: "oklch(29.4% 0.0342 131.83)"
    quietInk: "oklch(48.42% 0.0377 126.87)"
    rule: "oklch(74.54% 0.055 119.43)"
    action: "oklch(86.84% 0.1617 96.9)"
    focus: "oklch(52.28% 0.1304 133.62)"
    success: "oklch(52.37% 0.1292 139.2)"
    error: "oklch(64.65% 0.1408 36.69)"
    composerInk: "oklch(40% 0.05 115)"
    coverRule: "oklch(100% 0 0 / 0.18)"
    coverRuleSoft: "oklch(100% 0 0 / 0.17)"
    coverRuleFaint: "oklch(100% 0 0 / 0.12)"
    deepShadow: "oklch(0% 0 0 / 0.34)"
    mediumShadow: "oklch(0% 0 0 / 0.22)"
    softShadow: "oklch(0% 0 0 / 0.08)"
    raisedPaper: "oklch(98% 0.03 100)"
    whitePaper: "oklch(99% 0.02 100)"
    coverText: "oklch(84% 0.06 116)"
    coverTextDim: "oklch(74% 0.07 116)"
    coverTextBright: "oklch(87% 0.055 116)"
    connectText: "oklch(88% 0.04 115)"
    brightSuccess: "oklch(78% 0.2 125)"
    actionHover: "oklch(82% 0.16 96.9)"
    successInk: "oklch(43% 0.11 137)"
    successDeep: "oklch(34% 0.1 139)"
    successWash: "oklch(91% 0.08 130)"
    errorInk: "oklch(37% 0.12 34)"
    infoInk: "oklch(60% 0.08 212)"
    composer: "oklch(95.03% 0.0942 98.1)"
    composerControl: "oklch(97% 0.09 97)"
    composerLink: "oklch(31% 0.09 137)"
    darkCover: "oklch(13.5% 0.024 180)"
    darkCoverDeep: "oklch(11% 0.02 180)"
    darkPaper: "oklch(20.5% 0.025 195)"
    darkPaperGreen: "oklch(24.5% 0.032 192)"
    darkSky: "oklch(34.5% 0.045 197)"
    darkInk: "oklch(93% 0.015 95)"
    darkQuietInk: "oklch(75.6% 0.02 112)"
    darkRule: "oklch(39% 0.045 192)"
    darkControl: "oklch(20.5% 0.025 195)"
    darkComposer: "oklch(38.8% 0.06 123)"
typography:
    display:
        fontFamily: "IBM Plex Sans, ui-sans-serif, system-ui, sans-serif"
        fontSize: "2.6rem"
        fontWeight: 600
        lineHeight: 1.08
        letterSpacing: "-0.035em"
    deskTitle:
        fontSize: "1.5rem"
    sectionTitle:
        fontSize: "1.25rem"
    title:
        fontFamily: "IBM Plex Sans, ui-sans-serif, system-ui, sans-serif"
        fontSize: "1.125rem"
        fontWeight: 600
        lineHeight: 1.3
    body:
        fontFamily: "IBM Plex Sans, ui-sans-serif, system-ui, sans-serif"
        fontSize: "1rem"
        fontWeight: 400
        lineHeight: 1.55
    code:
        fontFamily: "IBM Plex Mono, ui-monospace, monospace"
        fontSize: "0.875rem"
        fontWeight: 400
        lineHeight: 1.5
    micro:
        fontSize: "0.66rem"
    index:
        fontSize: "0.68rem"
    stamp:
        fontSize: "0.7rem"
    label:
        fontSize: "0.72rem"
    caption:
        fontSize: "0.75rem"
    metadata:
        fontSize: "0.78rem"
    compact:
        fontSize: "0.8rem"
    small:
        fontSize: "0.8125rem"
    brand:
        fontSize: "1.08rem"
    pageMinimum:
        fontSize: "1.75rem"
    connectMinimum:
        fontSize: "1.8rem"
    heroMinimum:
        fontSize: "2.2rem"
    heroMobile:
        fontSize: "2.25rem"
    connectMaximum:
        fontSize: "2.7rem"
    heroMaximum:
        fontSize: "4.6rem"
rounded:
    control: "2px"
    panel: "2px"
spacing:
    sm: "8px"
    md: "16px"
    lg: "24px"
    xl: "40px"
---

# Design system: Power Plant

## Overview

**Creative north star: "Repository case file"**

Power Plant presents local agent work as a stable index beside a continuous paper work sheet.

The interface gives each task a clear location, readiness state and next action. It feels precise, practical and lightly playful.

Springfield combines olive covers, pale yellow paper, sunshine actions and lime status marks. Muted sky blue remains an informational accent.

Evergreen Terrace combines deep teal surfaces with marigold actions and mint status marks.

Leftorium is a restrained warm neutral theme. Stonecutters uses deep blue surfaces and cool blue actions.

Sector 7-G uses deep violet surfaces, safety lime and reactor cyan.

## Surfaces

- The product offers five colour themes. Springfield is the default.
- The theme changes immediately and persists in local filesystem storage.
- Settings puts Theme first and the model catalogue refresh second.
- Settings links to Presets and Environments before the separate local data reset.
- Presets uses a ruled list and an inline editor with shared instruction, tool and environment presentation.
- Preset deletion has an inline confirmation. It leaves existing copies unchanged.
- Unavailable preset resources retain their values and explicit status labels.
- Conversation Settings shows preset provenance and local customisation. Manage presets opens the explicit source editor.
- Reset requires an explicit confirmation and records a restart-applied deletion.
- Project source directories outside the Power Plant data directory remain unchanged.
- The desktop shell uses a persistent index grouped into Work, History and System.
- Work contains Conversations and Workflows. History contains Runs. System contains Providers and Settings.
- The brand mark and the first index link go to `/conversations`.
- Providers uses `/connect` in the app shell after the first provider connects.
- Before the first provider connects, `/connect` uses the standalone setup introduction.
- The mobile shell uses a compact masthead with Conversations, Workflows, Runs and More.
- More contains Providers and Settings.
- Conversation history provides directory filters for work discovery. A filter grants no access.
- Catalogue pages use ruled records with direct labels, metadata and status marks.
- Workflow catalogue entries and configuration pages show the ordered process before edit fields.
- Workflow authoring offers Run once and For each task. A repeated group shows the per-task phases once. It does not duplicate them for every task.
- Workflow phase settings use a keyboard-accessible selector. Unselected panels stay in the form so unsaved values remain.
- Invalid controls reveal their phase before browser validation moves focus. Server errors reveal the first affected phase and its advanced settings.
- Workflow overviews show each phase purpose, candidate effect, fresh model context and approval route. Independent review is labelled only when that phase exists. Environment overrides and technical mappings stay under Advanced settings.
- Conversation workflow setup has Choose a workflow, Workflow inputs and Review workflow states on one canonical GET route.
- The chooser shows names and short outcomes. Only the selected process shows its ordered phases through the shared process template.
- Input controls show the selected process requirements. Model and preset overrides stay collapsed until requested or invalid.
- Review shows selected context, effects, effective access, environment readiness and approval stops before Start workflow.
- Workers receive selected inputs, not the entire conversation history. Setup navigation creates no run or reservation.
- Back controls retain entered values. A stale process selection returns to the chooser without a substitute.
- Conversation pages keep the transcript and composer within the viewport. The empty transcript offers a short invitation, not setup forms or task suggestions.
- Compact model, network, environment and directory summaries stay visible above the transcript. Project access remains separate when applicable.
- A separate Conversation settings button ends the summary row.
- Each summary opens its section in one centred Conversation settings panel.
- A left menu selects one section. Narrow viewports use a section dropdown.
- Directories and conversation details have separate sections. Presets apply across all settings.
- The active section has a tinted background and bold text. Arrow keys select adjacent sections.
- The model list overlays controls without a layout shift.
- Tab changes retain unsaved fields. Validation reveals hidden fields before focus moves.
- The panel keeps its heading, menu and footer visible. Its dimensions stay fixed within the viewport. The content area scrolls independently.
- Host command approval sits beside host-mode consent. Ask each time is the default. Run without approval needs a distinct destination consent.
- A host approval preference change does not approve a waiting command. Active work must finish or stop before the new execution configuration takes effect.
- Settings updates keep the panel open. Invalid updates reveal the panel and retain submitted fields.
- Execution setting switches stay inline in one Conversation settings panel. Directory rows contain their requested strategy selectors.
- Directory strategy selectors share the backend, approval policy and environment preview.
- An idle switch requires Change execution settings after the preview. A preview or failed settlement leaves effective summaries unchanged.
- Active work offers Stop task and switch. Reviewed work offers review or exact-candidate discard. Summaries keep the current effective settings until settlement succeeds.
- Switch warnings name temporary attempt loss. They state that conversation history, existing host files, direct writes and host side effects remain.
- New conversation settings remain draft values until a valid first message creates the conversation.
- The Conversation details section contains title and delete controls without a disclosure. Delete retains its separate confirmation.
- Documents opens a separate panel from the conversation header, with a count when documents exist. It never occupies transcript space.
- The document panel explains that saved plans and task lists support reuse in workflows. It opens automatically for document text errors.
- Completed responses keep one Save as… disclosure below their content. One title field serves both document actions.
- The expanded form explains plans and task lists. Both actions execute nothing.
- Conversation turn headings align the author and status vertically. Animated dots accompany Replying and respect reduced-motion preferences.
- Document views put Create task list or Run tasks before provenance. Export, revision history and independent review stay within the document view.
- Document import names the authorised directory and selected environment. It reads one file without model calls or task execution.
- Project access remains explicit. The composer shows each attached project and its access level.
- A centred transcript column aligns with the composer. Requests use a tinted surface. Replies use the paper surface.
- The composer expands with the message. Send sits beside the text area, with its keyboard shortcut below the button on desktop.
- Workflows… stays beside the conversation title.
- Jump to latest appears when the reader leaves the transcript end. New output does not move a reader away from earlier messages.
- Thinking visibility remains a local preference for shared chat controls.
- The thinking control lists only efforts that the selected model advertises. It labels the upstream `none` effort as Off.
- The thinking control shows Not available when no adjustable effort exists.
- First-time connect uses a dark setup introduction beside a pale provider file.
- Established provider management uses the standard app shell and marks Providers as active.
- Provider setup presents plan login and API key as separate connection methods.
- Connect setup labels are Connect a model, Start a conversation and Send a message.
- Forms use bordered field groups and dense controls without decorative cards.
- Agent configuration puts network access between tools and project access.
- Agent network access uses three direct choices: No network, Restricted domains and Public internet.
- Restricted network access uses one base domain per line. The form states that each entry includes subdomains and ports.

## Hierarchy

- Conversations come first in the product index.
- The page title and next action form the first visual level.
- Workflow process order and effects appear before configuration fields.
- On project detail, the project name is the title. The host path is quiet monospace metadata.
- Readiness states appear before task input.
- Send is the primary composer action.
- Workflows… sits beside the conversation title.
- Yellow identifies actions and the composer.
- Green identifies ready or connected states.
- Red identifies destructive actions and errors.
- Monospace text identifies file references, revisions and technical metadata.

## Rules

- Use DaisyUI primitives for buttons, fields, alerts and menus.
- Use Tailwind utilities in Askama templates for layout.
- Keep shared shell and material styles in `app/assets/input.css`.
- Use square corners, thin rules and restrained shadows.
- Use sky blue for slim file register strips and information.
- Keep Conversations as the first index link and the brand destination.
- Put Settings in the System group of the product index.
- Keep first-time `/connect` navigation as a standalone page.
- Use app-shell navigation for `/connect` when a provider already exists.
- Keep Theme as the first item on the Settings page.
- Put the local data reset in a separate danger section on Settings.
- Do not take a deletion path from the browser.
- Do not stop the server from the reset command.
- Keep mobile navigation labels generic. Use conversation history and its directory filters for work discovery.
- Do not put project names in the permanent mobile row.
- Keep the Settings shortcut visible in the conversation detail. Keep project access and workflow launch explicit.
- Plain chat needs no sandbox when no tools are selected.
- Advertise only selected tools. Reject calls for tools outside the effective selection.
- Show workflow process structure before model, environment and technical settings.
- Show a parent, task and phase hierarchy on task-loop runs and child runs.
- Keep Run once available without a task list. Keep ordinary chat free of a required workflow.
- Keep phase selection transient. Do not submit a phase switch or discard fields from unselected phases.
- Keep environment overrides and technical mappings inside Advanced settings.
- Reuse the workflow process presentation in the conversation launch sheet.
- Do not duplicate the Providers route beside conversation model controls.
- Keep Send as the primary conversation action.
- Keep Workflows… beside the conversation title.
- Keep native links as navigation fallbacks for Hypergraft routes.
- Keep focus indicators visible on every interactive control.
- Keep No network as the default for new agents.
- State that model requests stay outside the sandbox on the agent form.
- Respect reduced-motion preferences.
- Do not use title case for labels, buttons or headings.
