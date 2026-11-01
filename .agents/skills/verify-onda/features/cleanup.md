# Post-processing (cleanup)

Optional OpenRouter cleanup of transcripts, configured on the Post-Processing
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
5. Toggle off again before cleanup.

## Gotchas

- Each provider's API key has its own Keychain account (service
  `com.rselbach.onda`; OpenRouter's is `openRouterApiKey`), shared with the
  user's Onda. If one is stored, every
  dictation with post-processing on sends a real request. `doctor` warns and
  `launch --setting postProcessingEnabled=true` refuses when it exists. With
  a key present, verify only the page, toggle it back off, and never dictate
  while it is on.
- Never type a key, paste one, or press Test; never put the field's value in
  evidence.
- With no stored key, dictating with post-processing on is safe: CLN-2
  delivers the raw transcript and the entry's `metadata.json` records
  `postProcessingErrorDescription` "OpenRouter unavailable". Do not add a
  key to reach "Refining...".
- Request behavior (headers, prompt, guardrails) is covered by Rust tests in
  `crates/onda-cleanup`, not by this skill.
