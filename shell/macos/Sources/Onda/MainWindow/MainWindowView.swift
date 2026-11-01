import OndaCore
import SwiftUI

/// Sidebar navigation over the pages the core describes (WIN-1, WIN-2).
struct MainWindowView: View {
    let model: MainWindowModel
    let core: OndaCore
    let sidebar: [SidebarGroup]
    let actions: MainWindowActions

    var body: some View {
        NavigationSplitView {
            List(selection: selection) {
                ForEach(sidebar, id: \.title) { group in
                    Section(group.title) {
                        ForEach(group.pages, id: \.section) { page in
                            SidebarRow(page: page)
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 190, ideal: 210, max: 260)
        } detail: {
            detail
                .navigationTitle(title)
        }
        .frame(minWidth: 760, minHeight: 540)
    }

    private var selection: Binding<MainWindowSection?> {
        Binding(
            get: { model.selection },
            set: { page in
                if let page {
                    actions.select(page)
                }
            }
        )
    }

    private var title: String {
        sidebar.lazy
            .flatMap(\.pages)
            .first { $0.section == model.selection }?
            .title ?? ""
    }

    @ViewBuilder
    private var detail: some View {
        switch model.selection {
        case .gettingStarted:
            if let page = model.gettingStarted {
                GettingStartedPageView(page: page, core: core, tryItText: tryItText)
            }
        case .general:
            if let page = model.general {
                GeneralPageView(page: page, core: core)
            }
        case .hotkey:
            if let page = model.hotkeyPage, let hotkey = model.hotkey {
                HotkeyPageView(page: page, hotkey: hotkey, core: core)
            }
        case .speech:
            if let page = model.speech {
                SpeechPageView(page: page, core: core)
            }
        case .postProcessing:
            if let page = model.postProcessing {
                PostProcessingPageView(page: page, core: core)
            }
        case .replacements:
            if let page = model.replacements {
                ReplacementsPageView(page: page, core: core)
            }
        case .history:
            if let page = model.history {
                HistoryPageView(page: page, core: core)
            }
        case .about:
            if let page = model.about {
                AboutPageView(page: page, core: core)
            }
        }
    }

    private var tryItText: Binding<String> {
        Binding(get: { model.tryItText }, set: { model.tryItText = $0 })
    }
}

/// A sidebar entry with its colored rounded-square icon (WIN-2).
private struct SidebarRow: View {
    let page: SidebarPage

    var body: some View {
        Label {
            Text(page.title)
        } icon: {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 22, height: 22)
                .background(
                    RoundedRectangle(cornerRadius: MainWindowRadius.small)
                        .fill(color.gradient)
                )
        }
        .tag(page.section)
    }

    private var symbol: String {
        switch page.section {
        case .gettingStarted: "sparkles"
        case .general: "gearshape.fill"
        case .hotkey: "keyboard.fill"
        case .speech: "waveform"
        case .postProcessing: "wand.and.stars"
        case .replacements: "arrow.left.arrow.right"
        case .history: "clock.arrow.circlepath"
        case .about: "info"
        }
    }

    private var color: Color {
        switch page.section {
        case .gettingStarted: .green
        case .general: .gray
        case .hotkey: .indigo
        case .speech: .blue
        case .postProcessing: .purple
        case .replacements: .orange
        case .history: .teal
        case .about: .secondary
        }
    }
}
