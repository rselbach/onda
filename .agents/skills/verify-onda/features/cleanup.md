# Post-processing (cleanup)

Optional OpenRouter cleanup of transcripts, configured on the Post-Processing
page. Verify the UI only: no real API key, no requests.

## Sub-features

- `cln-toggle`: enable toggle; details hidden while off.
- `cln-key-field`: secure API key field with loading and error states.
- `cln-model-picker`: cleanup model pop-up.
- `cln-test`: Test button, result and error alert.
- `cln-overlay`: "Refining..." label and "Used Raw Transcript" notices
  during dictation.

## How to get to it (user POV)

Main window > Post-Processing.

## Driving it with verify-onda

1. Open the main window and select the `Post-Processing` row with
   `llama-cu`. Screenshot the off state.
2. Toggle on; screenshot the details (key field, model picker, Test).
   `onda-cli --data-dir <data> settings get postProcessingEnabled` is `true`.
3. Model picker: choose another model, then confirm the choice survives
   switching pages.
4. Toggle off again before cleanup.

## Gotchas

- The API key lives in the Keychain (service `com.rselbach.onda`, account
  `openRouterApiKey`), shared with the user's Onda. If one is stored, every
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
