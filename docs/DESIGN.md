# Agent Voice design

Reference: the existing desktop layout and VoltAgent's dark audio workspace
patterns. The user's latest direction is deep black with white controls and text.

- Canvas `#050505`; cards `#0C0C0C`; input surfaces `#101010`.
- Main text `#F5F5F5`; supporting text `#AAAAAA`; borders `#292929`.
- Primary actions use white `#F2F2F2` with black text `#080808`.
- Selections use neutral gray `#242424`; hover surfaces stay monochrome.
- Status badges communicate through explicit text, with white and gray styling.
- All interface colors are neutral grayscale. Render the supplied logo in white with a transparent background.
- Set the native caption and border to the same #050505 canvas, with #F5F5F5 caption text; reapply after system theme changes. Preserve standard system window buttons and dragging.
- Chinese UI text uses Microsoft YaHei UI. Use compact bold headings and legible regular body text.
- Use the supplied double-note chip artwork in white with transparency in the app header, taskbar and executable. Preserve its original under assets/source/chip-double-note-original.png; retain the previous source for history.
- Supply full, compact, horizontal, stacked and wordmark forms. The horizontal and stacked forms pair the complete artwork with “Agent Voice” in Segoe UI Semibold, all white.
- Use 4 px spacing increments, 16–24 px card padding, and 40 px action targets.
- Keep voice editing in the studio and host-file operations in integrations.
- A preview always names its subject: fixed voice sample, cursor line, or random event phrase.
- Keep long agent lists and integration content scrollable. Preserve access to actions at 940×620.
- Draft editing does not rewrite host settings. Host changes require a named connect, remove, or sync action.
- Voice profiles are user data and can all be deleted. Confirming deletion detaches and mutes affected host states.
- Each host has four independent event rows with a voice selector, phrase override, audio file and preview.
- Native windows scale their layout and fonts to fit compact displays; all actions remain reachable.
- Reconcile native controls by stable ID. Keep unchanged HWNDs and edit state; batch geometry updates and composite descendant paints. Repeated clicks on the current tab do not rebuild or repaint the page.
- Voice selection shows a memorable alias, documented gender and stable source ID. Aliases must never imply a real identity or an unverified character model.
- Voice profiles open on a small gallery of fixed voice and stable tuning combinations with separate preview/apply actions. Keep the full speaker selector in the editing tab; do not make random speaker selection the primary path.
- Voice editing has separate phrase and mixer panels. Use neutral native faders with visible numeric values, an explicit current preset, normalization state and reset. Keep preview actions visible in both panels.
- User tuning presets can be saved and shared. Apply them without replacing the selected voice, phrases, bindings or final volume. Clearly attribute official baseline parameters and application tuning recipes.
- Each delivered iteration updates its version. Cargo.toml is the single version source for the CLI, window, executable version resources, assembly manifest and portable package. Use a minor increment for new features and a patch increment for fixes; preserve prior releases.
- The default page is host selection, headed “选择宿主”. Its list contains actual harnesses, never voice profiles. Voice profiles are named “声音预设” in a separate page.
- At 16–24 px, use the supplied inner rounded chip frame and double note; at 32–64 px, include the chip pins. Use opaque, adjusted strokes. Keep the full circuit, waveform and binary artwork in the header and 96–256 px icon frames. Embed a 256 px header source for high-DPI rendering. Set separate title-bar and taskbar icons at their actual DPI sizes.
