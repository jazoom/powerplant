---
name: Circus
description: A dark local desk for a coding agent that conducts hosted models.
colors:
    ring: "oklch(82% 0.13 80)"
    paper: "oklch(24% 0.012 80)"
    ground: "oklch(16% 0.01 80)"
    rule: "oklch(32% 0.016 80)"
    ink: "oklch(94% 0.01 85)"
    quiet: "oklch(74% 0.018 80)"
    warning: "oklch(84% 0.12 85)"
    success: "oklch(78% 0.1 155)"
    error: "oklch(76% 0.13 25)"
typography:
    display:
        fontFamily: "IBM Plex Sans, ui-sans-serif, system-ui, sans-serif"
        fontSize: "2rem"
        fontWeight: 600
        lineHeight: 1.1
        letterSpacing: "-0.03em"
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
rounded:
    control: "2px"
    panel: "6px"
spacing:
    sm: "8px"
    md: "16px"
    lg: "24px"
    xl: "40px"
---

# Design system: Circus

## Overview

**Creative north star: "The ringmaster's desk"**

Circus is a local desk for a coding agent. The screen is dark. A gold ring marks the product. Message paper is warm and quiet. Action colour is scarce.

The character is precise and calm. It is not a chat toy and it is not a circus poster.

## Surfaces

- The product uses one dark theme.
- Connect is a single card on a dark ground.
- Chat is a full-height desk. The transcript scrolls. The composer stays at the bottom.

## Rules

- Use DaisyUI for buttons, fields, alerts and menus.
- Use Tailwind utilities in Askama templates for layout.
- Keep custom CSS for focus, markdown and shared chrome.
- Do not use title case on labels or headings.
