# Pet chat input focus reliability

## Goal

Make the desktop-pet chat composer reliably accept the first click and receive
keyboard focus whenever the chat window is opened. Improve the compact composer
layout by grouping Skills with the bottom controls and making the input area
comfortably multi-line, without changing session or window-placement behavior.

## Problem

The pet chat is a separate, always-on-top, non-activating native panel. On
macOS, the first click in an inactive non-activating panel may be consumed by
AppKit before WebView receives it. The chat window currently does not opt into
first-mouse delivery, unlike the pet launcher icon. In addition, the frontend
currently performs only one immediate focus attempt after the window is opened,
which can race the native show/focus transition.

## Design

1. Configure the pet-chat `WebviewWindowBuilder` with
   `accept_first_mouse(true)`. Tauri treats this as a no-op on platforms where
   the setting is irrelevant, so it is safe for Windows and Linux.
2. Centralize composer focusing in the pet-chat frontend. Focus immediately
   after Vue renders, then retry after the next animation frame and a short
   timeout while the popup is visible. This protects against WebView/native
   focus timing without polling or stealing focus later in a conversation.
3. On a pointer-down inside the composer, explicitly focus the textarea unless
   the pointer was on the send/stop button. This handles any remaining dropped
   browser-level focus event.
4. Move the Skills picker from its dedicated row above the textarea into the
   bottom action row, alongside attachments, project folder, permission, model,
   and reasoning controls. Preserve its existing popup behavior and selected
   count. The action row remains a single horizontal, clipped/scrollable control
   strip rather than creating another composer row.
5. Increase the textarea's resting height to approximately three lines (about
   64px) while retaining its existing maximum height and Enter-to-send behavior.
6. Keep existing keyboard handling, sending, toolbar popovers, and native
   window activation policy unchanged.

## Verification

- Add a focused unit-level regression check for the builder configuration or a
  small pure helper if needed; it must fail before the configuration change.
- Run the relevant frontend test suite and Rust formatter/check for the changed
  files.
- Manually verify on macOS: with the popup closed and RunJam unfocused, click
  the pet icon once, then click/type in the composer; repeat after hiding and
  reopening it. Confirm send and toolbar controls retain their existing
  behavior, and that Skills remains reachable from the bottom row.

## Scope and non-goals

This change only addresses focus and first-click delivery plus the pet chat
composer layout. It does not redesign the pet panel, change its
always-on-top/full-screen behavior, or modify agent/session lifecycle handling.
