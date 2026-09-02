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
    darkCover: "oklch(16.5% 0.023 122)"
    darkCoverDeep: "oklch(11.5% 0.018 124)"
    darkPaper: "oklch(22.5% 0.028 112)"
    darkPaperGreen: "oklch(26.5% 0.035 120)"
    darkSky: "oklch(32% 0.052 215)"
    darkInk: "oklch(91% 0.028 100)"
    darkQuietInk: "oklch(70% 0.035 108)"
    darkRule: "oklch(39% 0.042 115)"
    darkControl: "oklch(18.5% 0.021 116)"
    darkComposer: "oklch(30% 0.065 92)"
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

Springfield light combines olive covers, pale yellow paper, sunshine actions and lime status marks. Muted sky blue remains an informational accent.

Springfield dark moves the paper to deep olive. It keeps yellow actions, green status marks and a dark teal file bar.

## Surfaces

- The product offers Springfield light and Springfield dark. Springfield light is the default.
- The theme changes immediately and persists in browser storage.
- The desktop shell uses a persistent index for projects, agents, workflows, environments, runs and settings.
- The brand mark and the first index link go to `/projects`.
- The mobile shell changes the index to a compact masthead and horizontal navigation.
- The mobile row stays a generic product index. The Projects page is the project switcher.
- Catalogue pages use ruled records with direct labels, metadata and status marks.
- The project desk uses model controls, a readiness route, a transcript sheet and an attached yellow composer.
- Connect uses a dark setup introduction beside a pale provider file.
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
- Put Settings after the work catalogues in the product index.
- Keep Theme as the first item on the Settings page.
- Keep mobile navigation labels generic. Use the Projects page as the project switcher.
- Do not put project names in the permanent mobile row.
- Keep Quick task Send as the primary composer action.
- Keep configured workflows inside an advanced disclosure.
- Keep native links as navigation fallbacks for Hypergraft routes.
- Keep focus indicators visible on every interactive control.
- Respect reduced-motion preferences.
- Do not use title case for labels, buttons or headings.
