// 翻译: Chinese <-> English with Apple's Translation framework, on device.
// macOS 26 and later only. Called from Rust (services/translate.rs) through
// the C functions at the bottom.
//
// When the language pair is installed, a TranslationSession is made directly.
// When it still has to be downloaded, the system download prompt can only be
// shown through SwiftUI's .translationTask, so a 1×1 hosting view is attached
// to the main window and the job runs there.

import AppKit
import Foundation
import SwiftUI
import Translation

public typealias TranslateCallback = @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?) -> Void

/// Result codes, matched in translate.rs.
private enum Code: Int32 {
    case ok = 0
    case unsupportedOS = 1
    case unsupportedPair = 2
    case failed = 3
}

/// The Rust side's callback and its context, passed back exactly once.
private final class Reply: @unchecked Sendable {
    private let context: UnsafeMutableRawPointer?
    private let callback: TranslateCallback
    private var sent = false
    private let lock = NSLock()

    init(context: UnsafeMutableRawPointer?, callback: TranslateCallback) {
        self.context = context
        self.callback = callback
    }

    deinit { send(.failed, "cancelled") }

    var pending: Bool {
        lock.lock()
        defer { lock.unlock() }
        return !sent
    }

    func send(_ code: Code, _ text: String) {
        lock.lock()
        defer { lock.unlock() }
        guard !sent else { return }
        sent = true
        text.withCString { callback(context, code.rawValue, $0) }
    }

    func armTimeout() {
        DispatchQueue.global().asyncAfter(deadline: .now() + 60) { [weak self] in
            guard let self else { return }
            self.send(.failed, "timeout")
            if #available(macOS 26.0, *) {
                Task { @MainActor in DownloadHost.remove(for: self) }
            }
        }
    }
}

private struct Job {
    let text: String
    let source: Locale.Language
    let target: Locale.Language
    let reply: Reply
}

/// Line by line, so paragraphs and blank lines come back where they were.
@available(macOS 26.0, *)
private func translateLines(_ text: String, with session: TranslationSession) async throws -> String {
    var lines = text.components(separatedBy: "\n")
    var requests: [TranslationSession.Request] = []
    for (index, line) in lines.enumerated() where !line.trimmingCharacters(in: .whitespaces).isEmpty {
        requests.append(.init(sourceText: line, clientIdentifier: String(index)))
    }
    if requests.isEmpty { return text }
    for response in try await session.translations(from: requests) {
        if let id = response.clientIdentifier, let index = Int(id) {
            lines[index] = response.targetText
        }
    }
    return lines.joined(separator: "\n")
}

@available(macOS 26.0, *)
private func describe(_ error: Error) -> String {
    if error is CancellationError { return "cancelled" }
    if let translation = error as? TranslationError {
        // The prompt was dismissed, or the download did not finish.
        let text = String(describing: translation)
        if text.localizedCaseInsensitiveContains("cancel") { return "cancelled" }
        return translation.localizedDescription
    }
    let nsError = error as NSError
    if nsError.domain == NSCocoaErrorDomain && nsError.code == NSUserCancelledError { return "cancelled" }
    return error.localizedDescription
}

// MARK: - Download path (SwiftUI)

@available(macOS 26.0, *)
private struct DownloadHostView: View {
    let job: Job
    let configuration: TranslationSession.Configuration

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)
            .translationTask(configuration) { session in
                guard job.reply.pending else { return }
                do {
                    job.reply.send(.ok, try await translateLines(job.text, with: session))
                } catch {
                    job.reply.send(.failed, describe(error))
                }
                await DownloadHost.remove(for: job.reply)
            }
    }
}

@available(macOS 26.0, *)
@MainActor
private enum DownloadHost {
    static weak var hostView: NSView?
    static weak var activeReply: Reply?

    static func run(_ job: Job, in window: NSWindow) {
        guard job.reply.pending else { return }
        guard let content = window.contentView else {
            job.reply.send(.failed, "no_window")
            return
        }
        activeReply?.send(.failed, "cancelled")
        hostView?.removeFromSuperview()
        let configuration = TranslationSession.Configuration(source: job.source, target: job.target)
        let view = NSHostingView(rootView: DownloadHostView(job: job, configuration: configuration))
        view.frame = NSRect(x: 0, y: 0, width: 1, height: 1)
        activeReply = job.reply
        hostView = view
        content.addSubview(view)
    }

    static func remove(for reply: Reply) {
        guard activeReply === reply else { return }
        hostView?.removeFromSuperview()
        hostView = nil
        activeReply = nil
    }
}

// MARK: - C interface

private func language(_ tag: String) -> Locale.Language? {
    switch tag {
    case "zh-Hans": return Locale.Language(identifier: "zh-Hans")
    case "en": return Locale.Language(identifier: "en")
    default: return nil
    }
}

/// 1 when this system has the APIs used here.
@_cdecl("cp_translation_supported")
public func cp_translation_supported() -> Int32 {
    if #available(macOS 26.0, *) { return 1 }
    return 0
}

/// Translates `text` from `source` to `target` ("zh-Hans" or "en"). `view`
/// is the main-thread webview. `callback` runs once, on
/// any thread, with a result code and the translation or an error message.
@_cdecl("cp_translate")
public func cp_translate(
    _ text: UnsafePointer<CChar>,
    _ source: UnsafePointer<CChar>,
    _ target: UnsafePointer<CChar>,
    _ view: UnsafeMutableRawPointer?,
    _ context: UnsafeMutableRawPointer?,
    _ callback: TranslateCallback
) {
    let reply = Reply(context: context, callback: callback)
    guard #available(macOS 26.0, *) else {
        reply.send(.unsupportedOS, "unsupported_os")
        return
    }
    let input = String(cString: text)
    guard let from = language(String(cString: source)), let to = language(String(cString: target)) else {
        reply.send(.unsupportedPair, "unsupported_pair")
        return
    }
    guard let view,
          let window = Unmanaged<NSView>.fromOpaque(view).takeUnretainedValue().window,
          window.contentView != nil else {
        reply.send(.failed, "no_window")
        return
    }
    let job = Job(text: input, source: from, target: to, reply: reply)
    reply.armTimeout()

    Task { @MainActor [weak window] in
        let status = await LanguageAvailability().status(from: from, to: to)
        guard reply.pending else { return }
        switch status {
        case .installed:
            Task.detached {
                do {
                    let session = TranslationSession(installedSource: from, target: to)
                    reply.send(.ok, try await translateLines(input, with: session))
                } catch {
                    reply.send(.failed, describe(error))
                }
            }
        case .supported:
            guard let window else {
                reply.send(.failed, "no_window")
                return
            }
            DownloadHost.run(job, in: window)
        default:
            reply.send(.unsupportedPair, "unsupported_pair")
        }
    }
}

/// 0 installed, 1 needs a download, 2 not available, 3 old system.
@_cdecl("cp_translation_status")
public func cp_translation_status(
    _ source: UnsafePointer<CChar>,
    _ target: UnsafePointer<CChar>,
    _ context: UnsafeMutableRawPointer?,
    _ callback: TranslateCallback
) {
    let reply = Reply(context: context, callback: callback)
    guard #available(macOS 26.0, *) else {
        reply.send(.ok, "3")
        return
    }
    guard let from = language(String(cString: source)), let to = language(String(cString: target)) else {
        reply.send(.ok, "2")
        return
    }
    reply.armTimeout()
    Task.detached {
        switch await LanguageAvailability().status(from: from, to: to) {
        case .installed: reply.send(.ok, "0")
        case .supported: reply.send(.ok, "1")
        default: reply.send(.ok, "2")
        }
    }
}
