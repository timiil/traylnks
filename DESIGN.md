---
version: alpha
name: TrayLnks
description: Windows tray launcher and terminal attention utility
colors:
  background: '#1e1e2e'
  panel: '#2a2a3c'
  primary: '#6b8cff'
  text: '#e4e4ef'
  muted: '#9a9ab0'
  border: '#3a3a52'
  inset: '#15151f'
  chrome: '#181828'
typography:
  sans:
    fontFamily: 'Segoe UI, system-ui, sans-serif'
    fontSize: '14px'
  mono:
    fontFamily: 'Cascadia Code, Consolas, monospace'
    fontSize: '12px'
rounded:
  DEFAULT: '6px'
spacing:
  page: '20px'
  field: '18px'
components:
  button:
    borderRadius: '6px'
  input:
    borderRadius: '6px'
---

# TrayLnks interface context

## Overview

A compact Windows utility for people launching local workspaces and watching AI terminal sessions. Preserve the existing dark, instrument-panel appearance: quiet surfaces, restrained blue actions, and copyable technical information. Settings uses English; snapshot content and documentation retain their established bilingual language. No particular geographic market is assumed.

The distinctive monitoring element is a short character change in the original terminal tab, followed by a static status symbol. Attention is informative, never a persistent animation. No marketing hero, decorative dashboard metrics, or new visual theme is needed.

Runtime ownership: `frontend/styles.css` is canonical for settings tokens and controls; this document records its values. ViewSnaps retains its denser existing viewer layout. New settings surfaces reuse the settings palette and control rules.

## Colors

Background, panel, inset, and chrome separate existing utility regions. Blue denotes a primary action or keyboard focus. Primary button text uses the dark background color for contrast. Monitoring states always have a symbol and a written description; color alone never conveys completion or error.

## Typography

Segoe UI and system fallbacks own prose and controls. Cascadia Code and Consolas own copyable paths and commands. Use 14px body text, 13px navigation, and 12px technical/help text. Chinese and mixed-script terminal titles remain readable and wrap without exposing raw control characters.

## Layout

Keep the top navigation, independently scrolling main content, and stable Apply/Cancel footer. Main content uses 20px padding and fields use 18px spacing. The Monitoring form uses a responsive grid; it must not change other panels' scroll ownership. Session lists render in bounded batches of 30 with an explicit Show more action. Their state is transient in this local Tauri utility, rather than encoded in a URL containing session metadata.

## Elevation & Depth

Use tonal surfaces and one-pixel borders. No new shadows or overlays. Setup instructions use an inline disclosure.

## Shapes

Retain 6px control corners and existing modest button padding. Scrollbars remain visible with a shared baseline and forced-colors fallback.

## Components

Settings controls are a buffered draft: Apply commits and Cancel reloads the saved state before hiding. Session actions and preparing setup commands are immediate actions, matching existing action buttons. Status refreshes must never replace unsaved form values.

Use native number inputs and checkboxes, accepting Windows-owned interaction. Existing selects remain native. Validation is inline, associated with the input, and focuses the first invalid field without native validation bubbles. Shared feedback lives in stable status regions. Buttons have hover, focus-visible, pressed, disabled, and busy states; save controls retain their geometry while pending.

Loading, empty, failed-load, observer-unavailable, paused, and active session states are explicit. Session labels and errors use text nodes. Keyboard users can reach all controls and copyable command blocks.

## Do's and Don'ts

- Keep native terminal titles readable and preserve applications' latest base title.
- Offer zero-duration static markers for users who prefer no animation.
- Preserve per-turn suppression and avoid replaying alerts after reopening settings or restarting TrayLnks.
- Do not treat quiet output as proof of completion or disconnection.
- Do not save terminal text, prompts, or keystrokes for monitoring.
