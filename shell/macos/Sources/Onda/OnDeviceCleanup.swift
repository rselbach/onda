import Foundation
import FoundationModels
import OndaCore

/// Apple Intelligence's on-device model behind the core's Apple
/// Intelligence cleanup provider. The core decides the prompt, timeout, and
/// fallback; this file only runs the Foundation Models framework.
///
/// The core calls `respond` from its background threads and it blocks, so
/// the async work runs in a detached task while the caller waits.
final class OnDeviceCleanup: OnDeviceCleanupPort, Sendable {
    func availability() -> OnDeviceAvailability {
        switch SystemLanguageModel.default.availability {
        case .available:
            .available
        case .unavailable(.appleIntelligenceNotEnabled):
            .notEnabled
        case .unavailable(.modelNotReady):
            .notReady
        case .unavailable(.deviceNotEligible):
            .deviceNotEligible
        case .unavailable:
            .deviceNotEligible
        }
    }

    func respond(instructions: String, prompt: String) throws -> String {
        if Thread.isMainThread {
            throw OnDeviceError.Failed(message: "the on-device model was called on the main thread")
        }
        let outcome = Answer<Result<String, OnDeviceError>?>(nil)
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            do {
                let session = LanguageModelSession(instructions: instructions)
                let response = try await session.respond(
                    to: prompt,
                    options: GenerationOptions(temperature: 0)
                )
                outcome.set(.success(response.content))
            } catch {
                outcome.set(.failure(.Failed(message: error.localizedDescription)))
            }
            done.signal()
        }
        done.wait()
        guard let result = outcome.get() else {
            throw OnDeviceError.Failed(message: "the on-device model finished without a result")
        }
        return try result.get()
    }
}
