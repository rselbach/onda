import AppKit
import Observation
import OndaCore
import SwiftUI

/// What the onboarding window shows: the core's view model, the slide
/// direction it was rendered with, and the recorder's state.
@MainActor
@Observable
final class OnboardingModel {
    var view: OnboardingView?
    var forward = true
    var hotkey: HotkeyState?
    /// the fix whose grant the window is walking through in System Settings
    var guiding: PermissionFix?
}

/// The onboarding wizard window (ONB-1..ONB-13). Renders
/// `OnboardingChanged` and forwards every action to the core, which runs
/// the steps, the 1 s refresh, and completion.
@MainActor
final class OnboardingWindowController: NSObject, NSWindowDelegate {
    private static let stepAnimation = Animation.spring(duration: 0.35)

    private let core: OndaCore
    private let model = OnboardingModel()
    private let settingsGuide = SystemSettingsGuide()
    private var window: NSWindow?
    private var guideTask: Task<Void, Never>?
    private var closingFromCore = false
    /// a step change waiting for its slide direction to render first
    private var deferred: OnboardingView?

    init(core: OndaCore) {
        self.core = core
        super.init()
    }

    /// Renders the events this window owns; ignores the rest.
    func handle(_ event: CoreEvent) {
        switch event {
        case .openOnboarding:
            show()
        case .closeOnboarding:
            close()
        case let .onboardingChanged(view):
            render(view)
        case let .hotkeyChanged(hotkey):
            model.hotkey = hotkey
        default:
            break
        }
    }

    func windowWillClose(_: Notification) {
        endGuide()
        deferred = nil
        model.view = nil
        model.forward = true
        guard !closingFromCore else { return }
        // ONB-11: the core decides what an early close means, and ends a
        // running shortcut capture (HOT-8)
        core.onboardingClosed()
    }

    private func show() {
        let window = window ?? makeWindow()
        self.window = window
        if !window.isVisible {
            window.center()
        }
        // the 750 ms first-run open and "Run Setup Again" arrive through the core,
        // so the cooperative `activate()` would be refused
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }

    private func close() {
        guard let window, window.isVisible else { return }
        closingFromCore = true
        window.close()
        closingFromCore = false
    }

    /// steps slide in from the edge `forward` names (ONB-1); the direction
    /// renders before the step so the outgoing step leaves the same way
    private func render(_ view: OnboardingView) {
        let hadClipboardLink = (deferred ?? model.view)?.hasClipboardLink ?? false
        // a granted card drops its button, or switching to clipboard drops the
        // link, either of which ends the walk
        if let guiding = model.guiding,
           !view.offers(guiding) || (guiding == .openAccessibilitySettings && hadClipboardLink && !view.hasClipboardLink) {
            endGuide()
        }
        if deferred != nil {
            deferred = view
            return
        }
        let stepChanged = model.view.map { $0.step != view.step } ?? false
        guard stepChanged, window?.isVisible == true else {
            model.forward = view.forward
            model.view = view
            return
        }
        guard model.forward != view.forward else {
            withAnimation(Self.stepAnimation) {
                model.view = view
            }
            return
        }
        model.forward = view.forward
        deferred = view
        DispatchQueue.main.async { [weak self] in
            guard let self, let latest = deferred, window?.isVisible == true else { return }
            deferred = nil
            model.forward = latest.forward
            withAnimation(Self.stepAnimation) {
                self.model.view = latest
            }
        }
    }

    private func fix(_ button: PermissionFixButton) {
        endGuide()
        core.fixPermission(fix: button.fix)
        guard let guide = button.guide, let window else { return }
        model.guiding = button.fix
        guideTask = Task {
            await settingsGuide.begin(beside: window, dropsApp: guide.acceptsDroppedApp)
        }
    }

    private func endGuide() {
        guideTask?.cancel()
        guideTask = nil
        model.guiding = nil
        settingsGuide.end()
    }

    private func makeWindow() -> NSWindow {
        let root = OnboardingRootView(model: model, core: core) { [weak self] button in
            self?.fix(button)
        }
        let hosting = NSHostingController(rootView: root)
        hosting.sizingOptions = []
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 720, height: 560),
            styleMask: [.titled, .closable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Set Up Onda"
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true
        window.identifier = NSUserInterfaceItemIdentifier("com.rselbach.onda.onboarding")
        window.contentViewController = hosting
        window.setContentSize(NSSize(width: 720, height: 560))
        window.isReleasedWhenClosed = false
        window.delegate = self
        return window
    }
}

/// The wizard's frame: progress bar, the current step, and the footer
/// (ONB-1).
struct OnboardingRootView: View {
    let model: OnboardingModel
    let core: OndaCore
    let fix: (PermissionFixButton) -> Void

    var body: some View {
        VStack(spacing: 0) {
            if let view = model.view {
                OnboardingProgressBar(progress: view.progress)
                    .padding(.top, 24)
                    .padding(.horizontal, 180)
                OnboardingStepView(
                    content: view.content,
                    hotkey: model.hotkey,
                    core: core,
                    guiding: model.guiding,
                    fix: fix
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.horizontal, 48)
                .padding(.bottom, 12)
                .id(view.step)
                .transition(transition)
                OnboardingFooter(view: view, core: core)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 20)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipped()
        .background(OnboardingBackground())
    }

    private var transition: AnyTransition {
        let entering: Edge = model.forward ? .trailing : .leading
        let leaving: Edge = model.forward ? .leading : .trailing
        return .asymmetric(
            insertion: .move(edge: entering).combined(with: .opacity),
            removal: .move(edge: leaving).combined(with: .opacity)
        )
    }
}

/// Window color with two faint accent radial glows (ONB-1).
private struct OnboardingBackground: View {
    var body: some View {
        ZStack {
            Color(nsColor: .windowBackgroundColor)
            RadialGradient(
                colors: [Color.accentColor.opacity(0.12), .clear],
                center: UnitPoint(x: 0.5, y: 0.05),
                startRadius: 0,
                endRadius: 420
            )
            RadialGradient(
                colors: [Color.accentColor.opacity(0.05), .clear],
                center: UnitPoint(x: 0.15, y: 1.0),
                startRadius: 0,
                endRadius: 380
            )
        }
        .ignoresSafeArea()
    }
}

/// One capsule per step after Welcome; hidden on Welcome (ONB-1).
private struct OnboardingProgressBar: View {
    let progress: OnboardingProgress?

    var body: some View {
        HStack(spacing: 6) {
            ForEach(0 ..< Int(progress?.total ?? 0), id: \.self) { index in
                Capsule()
                    .fill(index < Int(progress?.filled ?? 0) ? Color.accentColor : Color.primary.opacity(0.12))
            }
        }
        .frame(height: 4)
        .opacity(progress == nil ? 0 : 1)
        .animation(.easeInOut(duration: 0.25), value: progress)
        .accessibilityElement()
        .accessibilityLabel("Setup progress")
        .accessibilityValue(progress.map { "\($0.filled) of \($0.total)" } ?? "")
        .accessibilityHidden(progress == nil)
    }
}

/// Back, the hint, and the primary button (default on Return).
private struct OnboardingFooter: View {
    let view: OnboardingView
    let core: OndaCore

    var body: some View {
        HStack {
            Button(view.backTitle) {
                core.onboardingBack()
            }
            .controlSize(.large)
            .opacity(view.backVisible ? 1 : 0)
            .disabled(!view.backVisible)
            .accessibilityHidden(!view.backVisible)
            if let download = view.download {
                OnboardingDownloadStrip(download: download)
                    .padding(.leading, 12)
                    .transition(.opacity)
            }
            Spacer()
            if let hint = view.hint {
                Text(hint)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
                    .transition(.opacity)
            }
            Button(view.primaryTitle) {
                core.onboardingContinue()
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .disabled(!view.primaryEnabled)
        }
        .animation(.easeInOut(duration: 0.2), value: view.hint)
        .animation(.easeInOut(duration: 0.2), value: view.download == nil)
    }
}

/// The selected model's download while it runs past the model step.
private struct OnboardingDownloadStrip: View {
    let download: OnboardingDownload

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(download.label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .monospacedDigit()
            ProgressView(value: download.progress)
                .progressViewStyle(.linear)
                .controlSize(.small)
        }
        .frame(width: 210)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(download.label)
    }
}

private extension OnboardingView {
    /// some card on screen still offers `fix`
    func offers(_ fix: PermissionFix) -> Bool {
        guard case let .permissions(_, microphone, typingAccess, _) = content else { return false }
        return [microphone, typingAccess].contains { $0.button?.fix == fix }
    }

    var hasClipboardLink: Bool {
        guard case let .permissions(_, _, _, clipboardLink) = content else { return false }
        return clipboardLink != nil
    }
}
