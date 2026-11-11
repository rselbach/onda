# Post-processing (cleanup)

Optional OpenRouter, Fireworks AI, or on-device Apple Intelligence cleanup
of transcripts, configured on the Post-Processing
page. Verify the UI only: no real API key, no requests.

## Sub-features

- `cln-toggle`: enable toggle; details hidden while off.
- `cln-model-row`: clickable "Model" row naming the model and provider,
  with a chevron.
- `cln-model-sheet`: the sheet the Model row opens: provider sidebar, the
  provider's secure API key field (loading and error states), a searchable
  model list where one click uses a model (checkmark), one "Search or
  enter a model ID" field that filters the list or offers "Use as model
  ID", "Refresh Models" with a spinner and red error, and "Done".
- `cln-prompt`: "Prompt" section naming the prompt in use ("Onda default"
  or "Custom prompt") over a three-line preview, and the "Post-processing
  prompt" sheet it opens: editor, "Onda default"/"Custom" badge, status
  line and character count, "What the model receives", "Restore Default",
  "Cancel", "Save". One prompt serves every provider and model.
- `cln-test`: Test button, result and error alert.
- `cln-overlay`: "Refining..." label and "Used Raw Transcript" notices
  during dictation.

## How to get to it (user POV)

Main window > Post-Processing.

## Driving it with verify-onda

1. Open the main window and select the `Post-Processing` row with
   `llama-cu`. Screenshot the off state.
2. Toggle on; screenshot the details (Model row, Test).
   `onda-cli --data-dir <data> settings get postProcessingEnabled` is `true`.
3. Click the Model row; screenshot the sheet. The model list needs
   network access to `https://openrouter.ai/api/v1/models` (no key is
   sent). Click a listed model: the checkmark moves and `openRouterModel`
   in `<data>/settings.json` shows its ID (`onda-cli settings get` does not
   expose it).
4. Type `greendale/human-being` in the search field and click the "Use as
   model ID" row that appears; it shows
   under "Custom" with the checkmark. Click "Done"; the Model row names it
   and the choice survives switching pages.
5. Click the Prompt row; screenshot the sheet. Replace the editor's text
   with `llama-cu set-value` on `text area "Prompt instructions"`: the
   badge, status, and count follow, and `settings.json` gains no
   `cleanupPrompt` until "Save". "Cancel" and Escape discard the draft. An
   empty or whitespace-only editor shows "Enter a prompt or restore the
   default." in red with "Save" disabled. "Save" closes the sheet, the row
   reads "Custom prompt", and `cleanupPrompt` in `<data>/settings.json`
   holds the text as typed. "Restore Default" then "Save" removes the key.
6. Toggle off again before cleanup.

## Gotchas

- Each provider's API key has its own Keychain account (service
  `com.rselbach.onda`: `openRouterApiKey`, `fireworksApiKey`), shared with
  the user's Onda. If one is stored, every
  dictation with post-processing on sends a real request. `doctor` warns and
  `launch --setting postProcessingEnabled=true` refuses when it exists. With
  a key present, verify only the page, toggle it back off, and never dictate
  while it is on.
- Apple Intelligence uses no key and sends nothing off the Mac, so it is
  the one provider a run may dictate with while post-processing is on. It
  appears only on a Mac that can run it; its pane shows a status line when
  it is turned off or still getting ready, and dictation then delivers the
  raw transcript with history note "Apple Intelligence unavailable".
- Fireworks AI lists its models only with its key, so with no key stored its
  sheet shows "Enter your Fireworks AI API key to see its models." and only
  the curated models. That is expected; do not add a key to get the list.
- Never type a key, paste one, or press Test; never put the field's value in
  evidence.
- With no stored key, dictating with post-processing on is safe: CLN-2
  delivers the raw transcript and the entry's `metadata.json` records
  `postProcessingErrorDescription` "OpenRouter unavailable". Do not add a
  key to reach "Refining...".
- The prompt sheet does not take background mouse clicks or `type-text`;
  press its buttons by element (accessibility press) and edit with
  `set-value`. A fresh `get-ax-state` is needed after the sheet opens, or
  the button ids are stale.
- A saved prompt is sent with every later dictation, so restore the default
  before any run that dictates with post-processing on.
- Request behavior (headers, prompt, guardrails) is covered by Rust tests in
  `crates/onda-cleanup`, not by this skill.
