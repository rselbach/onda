import os

/// Unified logging under the app's subsystem, one category per component
/// (LOG-1). Transcript text is never logged (LOG-2).
enum Log {
    static let subsystem = "com.rselbach.onda"
    static let app = Logger(subsystem: subsystem, category: "App")
    static let hotkey = Logger(subsystem: subsystem, category: "Hotkey")
    static let media = Logger(subsystem: subsystem, category: "Media")
    static let output = Logger(subsystem: subsystem, category: "Output")
    static let overlay = Logger(subsystem: subsystem, category: "Overlay")
    static let speech = Logger(subsystem: subsystem, category: "Speech")
    static let system = Logger(subsystem: subsystem, category: "System")
    static let updates = Logger(subsystem: subsystem, category: "Updates")
}
