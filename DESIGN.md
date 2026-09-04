---
name: Power Plant
description: A local coding agent desk that presents work as an indexed repository case file.
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
- Settings has a separate local data reset after Theme.
- Reset requires an explicit confirmation and records a restart-applied deletion.
- Project source directories outside the Power Plant data directory remain unchanged.
- The desktop shell uses a persistent index grouped into Work, Configure and System.
- Work contains Projects and Runs. Configure contains Agents, Workflows and Environments. System contains Providers and Settings.
- The brand mark and the first index link go to `/projects`.
- Providers uses `/connect` with full-page native navigation.
- The mobile shell changes the index to a compact masthead and a primary row of Projects, Runs and More.
- More contains Agents, Workflows, Environments, Providers and Settings.
- The mobile row stays a generic product index. The Projects page is the project switcher.
- Catalogue pages use ruled records with direct labels, metadata and status marks.
- The project desk uses visible provider, model and thinking controls, a readiness route, a transcript sheet and an attached yellow composer.
- Connect uses a dark setup introduction beside a pale provider file.
- Connect setup labels are Connect a model, Add a project and Send a task.
- Forms use bordered field groups and dense controls without decorative cards.

## Hierarchy

- Projects come first in the product index.
- The page title and next action form the first visual level.
- On the project desk, the project name is the title. The host path is quiet monospace metadata.
- Readiness states appear before task input.
- Quick task Send is the primary composer action.
- Configured workflows sit in an advanced disclosure on the same composer.
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
- Keep Projects as the first index link and the brand destination.
- Put Settings in the System group of the product index.
- Keep `/connect` as full-page native navigation.
- Keep Theme as the first item on the Settings page.
- Put the local data reset in a separate danger section on Settings.
- Do not take a deletion path from the browser.
- Do not stop the server from the reset command.
- Keep mobile navigation labels generic. Use the Projects page as the project switcher.
- Do not put project names in the permanent mobile row.
- Keep the provider, model and thinking controls visible on the project desk.
- Do not duplicate the Providers route beside the project desk controls.
- Keep Quick task Send as the primary composer action.
- Keep configured workflows inside an advanced disclosure.
- Keep native links as navigation fallbacks for Hypergraft routes.
- Keep focus indicators visible on every interactive control.
- Respect reduced-motion preferences.
- Do not use title case for labels, buttons or headings.
