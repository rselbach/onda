@preconcurrency import AVFAudio
import Foundation
import OndaCore
import os
import Speech

/// The macOS speech recognizer behind the core's System Speech engine
/// (SYS-1..SYS-7). The core decides which locale to prepare, when, and what
/// errors mean; this file only runs the Speech framework.
///
/// SpeechTranscriber is preferred; DictationTranscriber (with punctuation)
/// covers the locales only it supports (SYS-4). The core calls `prepare`
/// and `transcribe` from its background threads and they block, so each
/// runs its async work in a detached task and waits for it.
final class SystemSpeech: SystemSpeechPort, Sendable {
    /// Jabber's list for macOS (SYS-1); the OS has the final say at load.
    private static let languages = [
        "ar", "bn", "ca", "cs", "da", "de", "el", "en", "es", "fi", "fr", "gu", "he", "hi",
        "hr", "hu", "id", "it", "ja", "kn", "ko", "ks", "mai", "ml", "mr", "ms", "nb", "ne",
        "nl", "or", "pa", "pl", "pt", "ro", "ru", "sk", "sv", "ta", "te", "th", "tr", "uk",
        "ur", "vi", "yue", "zh"
    ]

    /// The audio the core sends: 16 kHz mono Float32.
    private static let inputFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: 16000,
        channels: 1,
        interleaved: false
    )

    /// How often asset-download progress is reported.
    private static let progressInterval = Duration.milliseconds(100)

    /// What each prepared locale runs with, by the identifier the core
    /// sent. The core runs one engine call at a time (ARCH-8).
    private let cache = Answer<[String: Prepared]>([:])

    func details() -> SystemSpeechDetails {
        SystemSpeechDetails(
            description: "Built-in macOS speech recognition — no download required",
            languages: Self.languages,
            licenseName: "Apple System",
            licenseUrl: "https://www.apple.com/legal/sla/",
            attribution: "Apple Speech Framework"
        )
    }

    func currentLocale() -> String {
        Locale.current.identifier
    }

    func prepare(locale: String, progress: SpeechProgress) throws {
        try Self.blocking {
            _ = try await self.prepareLocale(locale, progress: progress)
        }
    }

    func transcribe(samples: [Float], locale: String) throws -> [String] {
        try Self.blocking {
            let entry = try await self.cachedOrPrepared(locale)
            return try await Self.finalResults(of: samples, with: entry)
        }
    }

    // MARK: - Preparation

    /// The Apple module that serves a locale, with the supported locale it
    /// runs as.
    private enum Transcriber: Sendable {
        case speech(Locale)
        case dictation(Locale)

        func makeModule() -> any SpeechModule {
            switch self {
            case let .speech(locale):
                SpeechTranscriber(
                    locale: locale,
                    transcriptionOptions: [],
                    reportingOptions: [],
                    attributeOptions: []
                )
            case let .dictation(locale):
                // unlike SpeechTranscriber, dictation punctuates only when
                // asked
                DictationTranscriber(
                    locale: locale,
                    contentHints: [],
                    transcriptionOptions: [.punctuation],
                    reportingOptions: [],
                    attributeOptions: []
                )
            }
        }
    }

    private struct Prepared: @unchecked Sendable {
        let transcriber: Transcriber
        /// What the analyzer wants; nil means feed the input as is.
        let format: AVAudioFormat?
    }

    private func cachedOrPrepared(_ identifier: String) async throws -> Prepared {
        if let entry = cache.get()[identifier] {
            return entry
        }
        return try await prepareLocale(identifier, progress: nil)
    }

    /// Picks the module, installs its assets when the OS lacks them, and
    /// remembers the analyzer format.
    private func prepareLocale(_ identifier: String, progress: SpeechProgress?) async throws -> Prepared {
        let clock = ContinuousClock()
        let started = clock.now
        let locale = Locale(identifier: identifier)
        let transcriber = try await Self.transcriber(for: locale)
        let module = transcriber.makeModule()
        // nil when the asset is already installed
        if let request = try await AssetInventory.assetInstallationRequest(supporting: [module]) {
            Log.speech.info("Downloading speech assets for \(identifier, privacy: .public)")
            let downloaded = request.progress
            let monitor = Task {
                while !Task.isCancelled, !downloaded.isFinished, !downloaded.isCancelled {
                    progress?.report(fraction: downloaded.fractionCompleted)
                    do {
                        try await Task.sleep(for: Self.progressInterval)
                    } catch {
                        break
                    }
                }
            }
            defer { monitor.cancel() }
            try await request.downloadAndInstall()
            progress?.report(fraction: 1)
        }
        let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [module])
        let entry = Prepared(transcriber: transcriber, format: format)
        var entries = cache.get()
        entries[identifier] = entry
        cache.set(entries)
        Log.speech.info(
            "Prepared \(identifier, privacy: .public) as \(String(describing: transcriber), privacy: .public) in \(String(describing: clock.now - started), privacy: .public)"
        )
        return entry
    }

    private static func transcriber(for locale: Locale) async throws -> Transcriber {
        if let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) {
            return .speech(supported)
        }
        if let supported = await DictationTranscriber.supportedLocale(equivalentTo: locale) {
            return .dictation(supported)
        }
        let name = Locale.current.localizedString(forIdentifier: locale.identifier) ?? locale.identifier
        throw SpeechError.UnsupportedLocale(displayName: name)
    }

    // MARK: - Transcription

    /// Runs one analysis over the whole recording and returns its final
    /// results in order; the core joins them (SYS-7).
    private static func finalResults(of samples: [Float], with entry: Prepared) async throws -> [String] {
        let buffer = try analyzerBuffer(samples, format: entry.format)
        let module = entry.transcriber.makeModule()
        let results: Task<[String], any Error> = switch module {
        case let speech as SpeechTranscriber: finalTexts(speech.results)
        case let dictation as DictationTranscriber: finalTexts(dictation.results)
        default: Task { throw SpeechError.Failed(message: "unexpected speech module") }
        }
        let (input, continuation) = AsyncStream<AnalyzerInput>.makeStream()
        let analyzer = SpeechAnalyzer(modules: [module])
        do {
            try await analyzer.start(inputSequence: input)
            continuation.yield(AnalyzerInput(buffer: buffer))
            continuation.finish()
            try await analyzer.finalizeAndFinishThroughEndOfInput()
            return try await results.value
        } catch {
            continuation.finish()
            results.cancel()
            await analyzer.cancelAndFinishNow()
            throw error
        }
    }

    private static func finalTexts<Results: AsyncSequence & Sendable>(
        _ results: Results
    ) -> Task<[String], any Error> where Results.Element: TranscribedText {
        Task {
            var texts: [String] = []
            for try await result in results where result.isFinal {
                texts.append(String(result.text.characters))
            }
            return texts
        }
    }

    /// The samples as one buffer in the analyzer's format.
    private static func analyzerBuffer(_ samples: [Float], format: AVAudioFormat?) throws -> AVAudioPCMBuffer {
        guard let inputFormat = Self.inputFormat,
              let input = AVAudioPCMBuffer(pcmFormat: inputFormat, frameCapacity: AVAudioFrameCount(samples.count)),
              let channel = input.floatChannelData
        else {
            throw SpeechError.Failed(message: "could not allocate the audio buffer")
        }
        input.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { source in
            if let base = source.baseAddress {
                channel[0].update(from: base, count: samples.count)
            }
        }
        guard let format, format != inputFormat else {
            return input
        }
        return try convert(input, to: format)
    }

    private static func convert(_ input: AVAudioPCMBuffer, to format: AVAudioFormat) throws -> AVAudioPCMBuffer {
        guard let converter = AVAudioConverter(from: input.format, to: format) else {
            throw SpeechError.Failed(message: "no converter to \(format)")
        }
        let ratio = format.sampleRate / input.format.sampleRate
        // room for the resampler's filter tail
        let capacity = AVAudioFrameCount((Double(input.frameLength) * ratio).rounded(.up)) + 4096
        guard let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else {
            throw SpeechError.Failed(message: "could not allocate the converted buffer")
        }
        let consumed = Answer(false)
        var conversionError: NSError?
        let status = converter.convert(to: output, error: &conversionError) { _, status in
            if consumed.get() {
                // one buffer only: end of stream (not .noDataNow) so the
                // resampler flushes its tail instead of dropping the last
                // milliseconds
                status.pointee = .endOfStream
                return nil
            }
            consumed.set(true)
            status.pointee = .haveData
            return input
        }
        if status == .error {
            throw SpeechError.Failed(
                message: "audio conversion failed: \(conversionError?.localizedDescription ?? "unknown")"
            )
        }
        return output
    }

    // MARK: - Bridging

    /// Runs `work` to completion and blocks the calling core thread until
    /// it finishes. Never called on the main thread: the core calls ports
    /// from its own threads, and waiting here would stall the UI.
    private static func blocking<Value: Sendable>(
        _ work: @escaping @Sendable () async throws -> Value
    ) throws -> Value {
        if Thread.isMainThread {
            throw SpeechError.Failed(message: "system speech called on the main thread")
        }
        let outcome = Answer<Result<Value, SpeechError>?>(nil)
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            do {
                let value = try await work()
                outcome.set(.success(value))
            } catch {
                outcome.set(.failure(Self.speechError(error)))
            }
            done.signal()
        }
        done.wait()
        guard let result = outcome.get() else {
            throw SpeechError.Failed(message: "system speech finished without a result")
        }
        return try result.get()
    }

    private static func speechError(_ error: any Error) -> SpeechError {
        switch error {
        case let error as SpeechError:
            return error
        case is CancellationError:
            return .Cancelled
        default:
            Log.speech.error("Speech framework failed: \(error.localizedDescription, privacy: .public)")
            return .Failed(message: error.localizedDescription)
        }
    }
}

/// The text both transcribers' results carry, so one collector serves
/// either module.
private protocol TranscribedText: SpeechModuleResult {
    var text: AttributedString { get }
}

extension SpeechTranscriber.Result: TranscribedText {}
extension DictationTranscriber.Result: TranscribedText {}
