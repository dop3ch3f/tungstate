import SwiftRs
import Tauri
import UIKit
import WebKit

// The iPhone side comes once Xcode is back on the build Mac; until then each
// call says so, rather than answering with nothing.
class PhotosPlugin: Plugin {
  @objc public func access(_ invoke: Invoke) throws {
    invoke.reject("The photo library is not read on iPhone yet.")
  }

  @objc public func ask(_ invoke: Invoke) throws {
    invoke.reject("The photo library is not read on iPhone yet.")
  }

  @objc public func summary(_ invoke: Invoke) throws {
    invoke.reject("The photo library is not read on iPhone yet.")
  }
}

@_cdecl("init_plugin_photos")
func initPlugin() -> Plugin {
  return PhotosPlugin()
}
