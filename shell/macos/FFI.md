# Onda FFI: what each window renders and calls

The Swift shell renders complete view models from `CoreEvent` and calls
`OndaCore` commands. It holds no rules (ARCH-3). Every type below lives in
`Sources/OndaCore/Generated/onda_ffi.swift`; Rust sources are
`crates/onda-ffi/src/{events,commands,ports,views}.rs` and
`crates/onda-app/src/views/`.

## General rules

- Events arrive in order on one core thread; hop to the main queue without
  reordering (`EventRelay`). Each page event replaces the whole page.
- Page events are sent only while that page is visible. Call
  `pageAppeared(page:)` when a page shows (the core re-reads what it needs
  and always re-sends the page) and `pageDisappeared(page:)` when it hides.
- Call `mainWindowAppeared()` when the main window shows and
  `mainWindowClosed()` when it closes. Closing flushes the Replacements and
  API key edits, cancels a cleanup test, and ends a recorder capture.
- `AppPresenceChanged(regular)`: set the activation policy (`.regular` /
  `.accessory`). The core counts the main window, onboarding, and the update
  notice (UI-2). The current shell only logs it, since it has no windows yet.
- `OpenMainWindow(page)`: show the main window (create once, keep it,
  UI-3) and select `page`, even if already open (UI-4).
- `ActivateApp`: bring Onda to the front (after a Keychain prompt, CLN-25).
- `ShowAlert(title, message, button)`: run a blocking modal alert with that
  one button (critical messages, 08; nothing in v1 sends one).
- `startFailureAlert()` gives the copy for the alert shown when
  `OndaCore.start` throws, before the core exists.
- Ids cross as strings (model ids, entry ids, rule ids, language codes,
  cleanup slugs). Send back only ids the core gave you.
- Dates: `HistoryRow.time`, `RecentTranscriptEntry.time`, and
  `GeneralPage.lastUpdateCheck` are `Date`s;
  `DayHeading.date` gives year/month/day for the shell to format
  (`inCurrentYear` → "Monday, October 27", else "December 3, 2025").
- Copy for every label, caption, and button is in the view models. Section
  headers that are only layout (e.g. "Permissions", "Try It") are the
  shell's.
- Never log `ApiKeyFieldView.text` (the Rust `Debug` redacts it; Swift's
  synthesized description does not).

## App menu and status menu

| Item | Renders | Calls |
| --- | --- | --- |
| Status menu | `MenuChanged(menu)`; rebuild on open | `menuWillOpen()` when it opens |
| Header | `menu.header` (disabled) | — |
| Fix Permissions… | `menu.fixPermissions` | `fixPermissions()` |
| Start/Stop/Cancel Dictation | `menu.dictation` | `toggleDictationFromMenu()` |
| Paste/Copy Last Transcript | `menu.lastTranscript` (subtitle) | `deliverLastTranscript()` |
| Recent Transcripts ▸ | `menu.recentTranscripts` items (title; subtitle `time.formatted(date: .abbreviated, time: .shortened)`, then " · " + `app` when non-nil), separator, `showAllTitle` | `deliverTranscript(text:)`, `showAllHistory()` |
| Retry Failed / Discard Failed | `menu.retryFailedDictation`, `discardFailedRecording` | `retryFailedDictation()`, `discardFailedDictation()` |
| Retry Loading Model / Re-download Model | `menu.retryModelLoad`, `redownloadModel` | `retryModelLoad()`, `redownloadModel()` |
| Language ▸ | `menu.language` (subtitle, checked items, `otherTitle`) | `selectLanguageFromMenu(code:)`, `chooseOtherLanguage()` |
| Open Onda / Settings… (⌘,) | `menu.openApp`, `menu.settings` | `openApp()`, `openSettings()` |
| Check for Updates… / Restart to Update / Download Onda … | `menu.update` (title, enabled, `action`) | `updateAction(action:)` |
| Quit (⌘Q) | `menu.quit` | `NSApp.terminate`; `shutdown()` in `applicationWillTerminate` |
| App menu "Settings…" (⌘,) | — | `openSettings()` |

Tray icon: `TrayChanged`. Recording overlay: `OverlayChanged`, cancel
button `cancelDictation()`; draw the waveform with
`waveformBars(levels:width:height:)` (bar width, spacing, and each bar's
height for the box, OVL-8), bars centered on the middle line. Download
overlay: `DownloadOverlayChanged`, close button `dismissDownloadOverlay()`.

## Main window pages

Sidebar order and sections come from `MainWindowSection`. Each page:

### Getting Started — `GettingStartedChanged(page: GettingStartedPage)`

- Checklist rows `steps` (icon from `status`), `readyLine` on top.
  Row button: `setupAction(action:)`.
- "Run Setup Again": `runSetupAgain()`.
- Try It: `tryItPrefix` + keycaps + `tryItSuffix`, field placeholder
  `tryItPlaceholder` (the field is a plain local text view).

### General — `GeneralChanged(page: GeneralPage)`

| Control | Calls |
| --- | --- |
| Permission rows `microphone`, `accessibility` (green `grantedLabel` or `fix` button; orange when `detailWarning`) | `fixPermission(fix:)` |
| Input picker `input` (`options`, `selectedUid`, orange `warning`, `caption`) | `selectInputDevice(uid:)` (`""` = System Default) |
| After transcription `output` | `setOutputMode(mode:)` |
| `pauseMedia` + `pauseMediaCaption` | `setPauseMedia(on:)` |
| `sounds` | `setSounds(on:)` |
| `launchAtLogin` + `launchAtLoginCaption` (tone, optional button) | `setLaunchAtLogin(on:)`, `openLoginItems()` |
| `automaticUpdates`, `update` (title, enabled, `action`) + `updateCaption` (tone), `versionFooter`, `lastUpdateCheck` | `setAutomaticUpdateChecks(on:)`, `updateAction(action:)` |

The device list updates live (the core hears device changes itself).

### Hotkey — `HotkeyChanged(hotkey: HotkeyState)` + `HotkeyPageChanged(page: HotkeyPage)`

- Keycaps, recorder button/help/message, Fn warning and its button:
  `HotkeyState`. Record button: `toggleHotkeyCapture()`; Cancel (shown while
  `recording`): `endHotkeyCapture()`. "Open Keyboard Settings":
  `openKeyboardSettings()`.
- Key monitor: one app-wide local monitor, installed exactly while
  `HotkeyState.listening` is true and removed when it turns false. Feed
  every key event to `recorderFlagsChanged(...)` / `recorderKeyDown(...)`
  (return value: swallow). If the monitor cannot be installed, call
  `hotkeyCaptureFailed()`. The Hotkey page and onboarding share it.
- `page.reset` ("Reset to ⌥ Space"): `resetHotkey()`. `page.footer`.
- Activation picker `activationOptions` / `activation` /
  `activationDescription`: `setActivationMode(mode:)`.
- Leaving the page or closing the window ends a capture in the core
  (HOT-8); the shell does nothing extra.

### Speech — `SpeechChanged(page: SpeechPage)`

- Language picker `languages` (draw a divider after an entry whose
  `dividerAfter` is set; `automatic` marks "Automatic"),
  `selectedLanguage`: `selectLanguage(code:)`.
- `compatibility` note (orange when `warning`); its `action` by `kind`:
  `.use` → `selectModel(id:)`, `.downloadAndUse` → `downloadAndUse(id:)`,
  `.downloading` disabled.
- Model rows `models`: `control` `.downloading` (bar, `percent`,
  `cancelDownload(id:)`), `.selected` (check), `.select`
  (`selectModel(id:)`), `.downloadAndUse` (`downloadAndUse(id:)`); trash when
  `deleteTooltip` is set: `requestDeleteModel(id:)`.
- `alert` (one at a time; the core queues them): `confirm` button →
  `confirmSpeechAlert(id:)`, `dismiss` button → `dismissSpeechAlert(id:)`.

### Post-Processing — `PostProcessingChanged(page: PostProcessingPage)`

- `enabled` toggle: `setCleanupEnabled(on:)`. `details` is nil while off.
- A clickable `modelLabel` row showing `modelName` over `providerName` and
  a chevron → `openProvidersDialog()`. The core lists the provider's
  models when the page shows with cleanup on, for the model's name.
- Present `dialog` as a sheet while it is non-nil; "Done" (`doneTitle`) or
  dismissing the sheet → `closeProvidersDialog()`.
- Sheet sidebar `dialog.providers` / `dialog.selectedProvider`:
  `selectDialogProvider(id:)`. Each entry's logo is the bundled
  `<id>-light.svg` / `<id>-dark.svg` or `<id>.svg` (assets/providers), else
  its `systemSymbol`; the entry with `inUse` carries a checkmark. The core
  leaves Apple Intelligence out on a Mac that cannot run it.
- The pane is headed by `dialog.providerName` over `dialog.privacyNote`.
- `dialog.statusMessage`, when set, says why the provider cannot run now
  (e.g. Apple Intelligence turned off); show it above the models.
- Sheet API key secure field `dialog.key` (nil for a provider without a
  key, such as Apple Intelligence), holding the key of the sheet's selected
  provider (each provider has its own): `label` beside it, `placeholder`
  inside it; show `text` when `revision` changes (never overwrite while
  typing); `loading`; red `error`. Under it, `note` and a link titled
  `getKeyTitle` that opens `getKeyUrl` in the browser. Every edit:
  `apiKeyEdited(text:)`. Return / end of editing: `apiKeyCommit()`.
  The core saves when the sheet closes, on page disappear, window close,
  and quit by itself. It reads the Keychain (and sends `ActivateApp`) only
  when the sheet opens or its provider changes.
- Sheet models `dialog.modelGroups` (a heading per group), each row its
  `name` followed by its `shortId` when set (the whole `slug` is the row's
  tooltip), with a checkmark and tint on `dialog.selectedModel` (nil when
  another provider is in use). Clicking a row →
  `selectCleanupModel(provider:slug:)` with the sheet's provider; that
  provider and model are used at once. One search field above the list
  (`searchPlaceholder`) filters rows by name or slug in the shell. When its
  trimmed text has no spaces and matches no row's slug, a row showing that
  text and `useIdCaption` heads the list; clicking it (or Return) sends
  `selectCleanupModel(provider:slug:)` with the text. `refreshModels` (nil
  for a provider with one fixed model, which also hides the search field)
  is an icon button beside the field, its title the tooltip →
  `refreshCleanupModels()`. While `modelsLoading`, show a `loadingTitle`
  row with a spinner above the list. `modelsNotice` goes under the search
  field: `.needsKey` (a provider that lists only with a key has none) in
  plain secondary text, `.failed` in red. The list can hold hundreds of
  rows: render it lazily (a `List`, not a `Form`).
- Sheet footer: `inUseLabel`, then `inUseModel` and `inUseProvider`, the
  model and provider cleanup uses now whichever pane shows, beside "Done".
- `prompt`, a section titled `prompt.title` with `prompt.caption` under
  it: a clickable row showing `name` over `subtitle` and a chevron, with
  `preview` below it in up to three lines → `openPromptEditor()`. One
  prompt serves every provider and model. Show `prompt.status`, when set,
  under the test button; the core clears it when the sheet opens again or
  the page disappears.
- Present `prompt.editor` as a sheet while it is non-nil. It holds a draft
  the core owns: show `text` in a large scrolling monospaced editor when
  `revision` changes (never overwrite while typing), and send every edit
  with `promptEdited(text:)`. `badge` beside `label`; `status` under the
  editor (red when `statusIsError`) with `characterCount` opposite it;
  `help` disclosed under `helpTitle`. `restore` →
  `restoreDefaultPrompt()`, which replaces the draft only. `cancelTitle`,
  Escape, or dismissing the sheet → `cancelPromptEditor()`, which
  discards the draft. `save` → `savePrompt()`; the core refuses a blank
  draft, stores nothing but the override (Onda's own text removes it), and
  closes the sheet, or keeps it open with `status` saying why the save
  failed.
- `testButton` + spinner while `testRunning`: `testCleanup()`.
  `testResult` (`said`, `result`, selectable); `testError` alert →
  `dismissCleanupTestError()`.
  Samples use the same output acceptance and retry rules as dictation.
  The default prompt keeps transcript-preservation checks. A custom prompt
  permits changes in length and formatting. Provider failures still fail
  the test, and rejected default-prompt output shows the fallback reason.

### Replacements — `ReplacementsChanged(page: ReplacementsPage)`

- Rows `rows` (`triggers` raw, `replacement`); rebind the text fields only
  when `revision` changes. Edits: `setReplacementTriggers(id:text:)`,
  `setReplacementText(id:text:)` on every keystroke. Trash:
  `deleteReplacementRule(id:)`. "Add Rule": `addReplacementRule()`.
- The core debounces writes (500 ms) and flushes on disappear, window close,
  and quit.

### History — `HistoryPageChanged(page: HistoryPage)`

| Control | Calls |
| --- | --- |
| `saveTranscripts` ("Turn On History" too) | `setHistoryEnabled(on:)` |
| Keep for (`retentionOptions`, `retention`, `retentionEnabled`) | `setHistoryRetention(retention:)` |
| `keepAudio` | `setHistoryKeepsAudio(on:)` |
| Search (`searchPrompt`, `query`) | `searchHistory(query:)` |
| `clearButton` / `clearTooltip` | `requestClearHistory()` |
| `dialog` (title, message, destructive `confirmTitle`, `cancelTitle`) | `confirmHistoryDialog()`, `cancelHistoryDialog()` |
| `error` alert ("History Error") | `dismissHistoryError()` |
| Row copy button (checkmark while `copiedEntry == row.id`) | `copyHistoryEntry(id:)` |
| Copy command (⌘C) | none: return `NSItemProvider(object: row.transcript ?? "")` for the selected row; `transcript` is already trimmed (HIS-25) |
| Row `menu` items by `item` | `.copy`, `.copyOriginal` → `copyHistoryOriginal(id:)`, `.playRecording` → `playHistoryRecording(id:)`, `.showRecording` → `showHistoryRecording(id:)`, `.divider`, `.delete` / Delete key → `deleteHistoryEntry(id:)` |

`content`: `.empty` (title, message, optional action), `.noSearchResults`
(the platform's "no results for 'query'" view), `.sections` (day headings,
rows). Row: `time` (localized short time) + " · " + `details`, `appId` for
the icon, `transcript` or italic `noTextLabel`, expandable `original` under
`originalLabel`. Selection and expansion are the shell's.

### About — `AboutChanged(page: AboutPage)`

App icon, `appName`, `version`, `tagline`, `models` (attribution + license
link), `libraries`.

## Onboarding window

- `OpenOnboarding`: create (once) and show the 720×560 window. Content:
  `OnboardingChanged(view: OnboardingView)`. `CloseOnboarding`: close it
  (after "Start Dictating"). If the user closes it: `onboardingClosed()`.
- Frame: `progress`, `backVisible`/`backTitle` → `onboardingBack()`, `hint`,
  `primaryTitle`/`primaryEnabled` → `onboardingContinue()` (Return); slide
  direction from `forward`.
- `content` by step:
  - `.welcome`: title, subtitle.
  - `.language`: search field → `onboardingSearchLanguages(query:)`, toggle
    → `onboardingToggleAllLanguages()`, cards → `onboardingSelectLanguage(code:)`.
  - `.permissions`: cards' buttons → `fixPermission(fix:)`, link →
    `onboardingUseClipboard()`.
  - `.hotkey`: presets → `onboardingSelectHotkey(preset:)`; the "Another
    shortcut" card uses the recorder (`HotkeyChanged` and the recorder
    commands, same key monitor as the Hotkey page; closing the window or
    leaving the step ends a capture in the core); Fn warning button →
    `openKeyboardSettings()`.
  - `.speechModel`: card actions `.download` → `onboardingDownloadModel(id:)`,
    `.useThisModel` → `onboardingChooseModel(id:)`, `.downloading` cancel →
    `cancelDownload(id:)`; other models toggle → `onboardingToggleOtherModels()`;
    red `error`.
  - `.ready`: prompt + keycaps, try-it placeholder, retention note.
- The core refreshes permissions, the hotkey, the Fn setting, and models
  every second while onboarding is open, and owns the 750 ms first-run
  delay. `uiReady()`: call when the first window appears, else on the next
  main-loop turn after launch (the current shell does the latter).

## Update notice window

`ShowUpdateNotice(notice: UpdateNotice)`: 440-pt window, title, selectable
Markdown `body` (bold model name), `buttons` left to right with `shortcut`
(Escape / Return, Return is the default). Any button or closing the window:
`updateNoticeChoice(choice:)` (close = `.notNow`). `CloseUpdateNotice`:
close it.

## Ports the shell implements (`ShellPorts`)

| Port | Methods |
| --- | --- |
| `TextOutputPort` | focused target, field text, `insert(text:target:delivery:)`, copy; `insert` calls `delivery.commit()` right before its first keystroke, accessibility write, or clipboard write and returns `.cancelled` with no output when it is false (DICT-19); `copy` must write at once, because the core commits right before calling it |
| `PermissionsPort` | microphone status/request, Accessibility trust/prompt, `openSettings(pane:)` (PERM-3 deep link, general pane fallback) |
| `SoundPort`, `PowerPort` | cues, idle-sleep prevention |
| `HotkeyPort` | register/unregister, Escape, stream re-enable, `globeKeyUsage()` (synchronize `com.apple.HIToolbox` before reading `AppleFnUsageType`, HOT-35), `openKeyboardSettings()` |
| `NotifierPort` | authorization, post, `requestAuthorization()` (the core decides when, ONB-12) |
| `FilesPort` | `excludeFromBackup`, `revealFile`, `openFile` |
| `LoginItemPort` | `status()` (nil when unbundled), `setEnabled`, `openSettings()` (`SMAppService`) |
| `UpdaterPort` | `openUrl(url:)` (the DMG of an update this copy cannot install), `terminate()` (`NSApp.terminate`; the core already swapped the bundle and a relauncher waits for the exit). The core checks, downloads, verifies, and installs (UPD-1..UPD-8). `ShellPorts.updateHost` describes the running bundle (path, `CFBundleVersion`, macOS version, isolated run, Sparkle's saved `SUEnableAutomaticChecks`); nil for a dev build, which never updates (UPD-4) |
| `MediaControlPort` (optional) | `nowPlaying()` → raw `NowPlaying` (`.failed(reason:)`, `.noTrack`, `.track(isPlaying:playbackRate:)`, each fact optional; the core decides what counts as playing), `pause`, `play`, `playPauseKey`, `muteAudibleOutput()` → the muted device or nil, `unmuteOutput(device:)`; blocking, called one at a time; nil disables media pause (MEDIA-1, MEDIA-5) |
| `SystemSpeechPort` (optional) | `details()` (catalog description, languages, license, attribution), `currentLocale()`, `prepare(locale:progress:)` (install assets, report download fractions to `SpeechProgress.report`), `transcribe(samples:locale:)` → final results; blocking, called one at a time from core threads, never the main thread; throws `SpeechError` (`UnsupportedLocale(displayName:)`, `Cancelled`, `Failed`); nil leaves System Speech out of the catalog (MOD-4, SYS-1..SYS-7) |
| `OnDeviceCleanupPort` (optional) | `availability()` → `OnDeviceAvailability` (`Available`, `DeviceNotEligible`, `NotEnabled`, `NotReady`), read fresh and cheap; `respond(instructions:prompt:)` → the model's text at temperature 0 in a new session; blocking, called from core threads, never the main thread; throws `OnDeviceError.Failed(message:)`, never with the transcript; nil leaves Apple Intelligence out of the cleanup providers |

`OndaCore.start(ports:dataDir:osLanguage:appVersion:trayTerm:privacyNaming:listener:)`;
`trayTerm` is what copy calls the tray icon ("menu bar icon" on macOS, 09
"Copy catalog"). `privacyNaming` is what the running OS calls the
Accessibility permission: `.deviceControlAndDataAccess` when
`ProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 27`, else
`.accessibility`. The core uses it for every string that names the
permission (setup and General rows, notices, the status menu header, the
tray label, onboarding's hint); the PERM-3 deep link is the same on both.
