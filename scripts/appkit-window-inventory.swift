import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2,
      let requestedPID = Int(CommandLine.arguments[1]) else {
    FileHandle.standardError.write(Data("usage: appkit-window-inventory PID\n".utf8))
    exit(2)
}

let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID)
        as? [[String: Any]] else {
    exit(1)
}

let matches = windows.compactMap { window -> [String: Any]? in
    let ownerPID = window[kCGWindowOwnerPID as String] as? Int ?? -1
    let title = window[kCGWindowName as String] as? String ?? ""
    let layer = window[kCGWindowLayer as String] as? Int ?? -1
    guard ownerPID == requestedPID,
          title == "Rinka Explorer" || title == "Connection Activity" else {
        return nil
    }
    return [
        "id": window[kCGWindowNumber as String] as? Int ?? 0,
        "pid": ownerPID,
        "title": title,
        "layer": layer,
        "bounds": window[kCGWindowBounds as String] as? [String: Any] ?? [:],
    ]
}.sorted {
    let leftTitle = $0["title"] as? String ?? ""
    let rightTitle = $1["title"] as? String ?? ""
    if leftTitle != rightTitle {
        return leftTitle < rightTitle
    }
    return ($0["id"] as? Int ?? 0) < ($1["id"] as? Int ?? 0)
}

let data = try JSONSerialization.data(
    withJSONObject: matches,
    options: [.prettyPrinted, .sortedKeys]
)
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write(Data("\n".utf8))
