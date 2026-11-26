import BackgroundTasks
import SwiftRs
import Tauri
import UIKit
import WebKit

class NoticeArgs: Decodable {
  let title: String
  let text: String
  let permille: Int
  let waiting: Bool
}

class AwakeArgs: Decodable {
  let on: Bool
}

/// Keeps a transfer the person started going after they leave the app.
///
/// iOS 26 and later: a continued processing task, made for exactly this. The
/// system shows its own progress card, and may end it if the phone needs the
/// time; that ending is passed to the run as Stop, which finishes the file in
/// hand, and the app offers Resume on return. Before 26: the few seconds iOS
/// grants every app on leaving (`beginBackgroundTask`), then the same Stop.
///
/// The transfer itself runs on the app's Rust threads; this only asks iOS for
/// the time. One at a time, as the engine runs them.
final class TransferKeeper {
  static let shared = TransferKeeper()

  /// The prefix registered, with a wildcard, in Info.plist's
  /// `BGTaskSchedulerPermittedIdentifiers`.
  private static let prefix = "com.ifeanyiibekie.tungstate.transfer"

  private let lock = NSLock()
  private var stopAsked = false
  private var grace: UIBackgroundTaskIdentifier = .invalid
  /// The continued processing task, once iOS has started it (iOS 26+).
  private var task: AnyObject?
  private var notice: NoticeArgs?

  func begin(_ first: NoticeArgs) {
    lock.lock()
    notice = first
    stopAsked = false
    lock.unlock()
    // The keeper lives as long as the app, so capturing it strongly is right.
    DispatchQueue.main.async {
      self.grace = UIApplication.shared.beginBackgroundTask(withName: "tungstate-transfer") {
        self.expired()
      }
    }
    if #available(iOS 26.0, *) {
      submit(first)
    }
  }

  /// Shows `next`, and answers whether the run should stop.
  func update(_ next: NoticeArgs) -> Bool {
    lock.lock()
    notice = next
    let stop = stopAsked
    stopAsked = false
    let running = task
    lock.unlock()
    if #available(iOS 26.0, *), let running = running as? BGContinuedProcessingTask {
      running.updateTitle(next.title, subtitle: next.text)
      running.progress.totalUnitCount = 1000
      running.progress.completedUnitCount = Int64(next.permille)
    }
    return stop
  }

  func end() {
    lock.lock()
    let running = task
    task = nil
    notice = nil
    lock.unlock()
    if #available(iOS 26.0, *), let running = running as? BGContinuedProcessingTask {
      running.setTaskCompleted(success: true)
    }
    DispatchQueue.main.async {
      if self.grace != .invalid {
        UIApplication.shared.endBackgroundTask(self.grace)
        self.grace = .invalid
      }
    }
  }

  /// iOS is taking the time back: stop after the file in hand.
  private func expired() {
    lock.lock()
    stopAsked = true
    lock.unlock()
    DispatchQueue.main.async {
      if self.grace != .invalid {
        UIApplication.shared.endBackgroundTask(self.grace)
        self.grace = .invalid
      }
    }
  }

  @available(iOS 26.0, *)
  private func submit(_ first: NoticeArgs) {
    // A fresh identifier per run: one registration per identifier is all
    // iOS allows, and the wildcard in Info.plist covers them all.
    let identifier = "\(Self.prefix).\(UUID().uuidString)"
    let registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: nil) { [weak self] started in
      guard let self, let started = started as? BGContinuedProcessingTask else { return }
      started.expirationHandler = { [weak self] in
        self?.expired()
        started.setTaskCompleted(success: false)
      }
      started.progress.totalUnitCount = 1000
      self.lock.lock()
      if self.notice == nil {
        // Ended before iOS got round to starting it.
        self.lock.unlock()
        started.setTaskCompleted(success: true)
        return
      }
      self.task = started
      self.lock.unlock()
    }
    guard registered else { return }
    let request = BGContinuedProcessingTaskRequest(identifier: identifier, title: first.title, subtitle: first.text)
    // Start now or not at all: a transfer queued for later is not what was asked.
    request.strategy = .fail
    do {
      try BGTaskScheduler.shared.submit(request)
    } catch {
      // Refused (too many, or not allowed): the grace period still holds,
      // and Resume picks up whatever it does not finish.
      NSLog("tungstate: could not keep the transfer going in the background: \(error)")
    }
  }
}

class PhotosPlugin: Plugin {
  // The photo library is read on Android; on iPhone it comes later, so each
  // call says so rather than answering with nothing.
  @objc public func access(_ invoke: Invoke) throws {
    invoke.reject("The photo library is not read on iPhone yet.")
  }

  @objc public func ask(_ invoke: Invoke) throws {
    invoke.reject("The photo library is not read on iPhone yet.")
  }

  @objc public func summary(_ invoke: Invoke) throws {
    invoke.reject("The photo library is not read on iPhone yet.")
  }

  /// Keeps the screen from locking while a run is open in front of you.
  @objc public func keepAwake(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(AwakeArgs.self)
    DispatchQueue.main.async {
      UIApplication.shared.isIdleTimerDisabled = args.on
      invoke.resolve()
    }
  }

  @objc public func runBegan(_ invoke: Invoke) throws {
    TransferKeeper.shared.begin(try invoke.parseArgs(NoticeArgs.self))
    invoke.resolve()
  }

  @objc public func runProgress(_ invoke: Invoke) throws {
    let stop = TransferKeeper.shared.update(try invoke.parseArgs(NoticeArgs.self))
    invoke.resolve(["stop": stop])
  }

  @objc public func runEnded(_ invoke: Invoke) throws {
    TransferKeeper.shared.end()
    invoke.resolve()
  }

  /// iOS shows the continued processing card without asking.
  @objc public func askNotifications(_ invoke: Invoke) throws {
    invoke.resolve()
  }
}

@_cdecl("init_plugin_photos")
func initPlugin() -> Plugin {
  return PhotosPlugin()
}
