import ExpoModulesCore
import UIKit
import UniformTypeIdentifiers

/// The system's paste button (UIPasteControl): a tap on it is the user's own consent, so iOS shows
/// no "Allow Paste" prompt and never refuses the read. Text arrives as onPasteText, an image as
/// base64 in onPasteImage: PNG, or JPEG when the PNG is over the upload limit. Anything else (a
/// Files copy) arrives as onPasteFile: its name and bytes in base64, or why it cannot go.
class RiftPasteButton: ExpoView {
  let onPasteText = EventDispatcher()
  let onPasteImage = EventDispatcher()
  let onPasteFile = EventDispatcher()

  /// Over this a screenshot goes as JPEG and a file not at all (MAX_UPLOAD_BYTES of the protocol).
  private static let maxPng = 10 * 1024 * 1024

  /// The icon alone (the key rows), or the icon with the system's word "Paste" across a row (the Attach sheet).
  var labelled = false {
    didSet {
      if labelled != oldValue {
        control.removeFromSuperview()
        control = Self.makeControl(labelled: labelled, target: self)
        addSubview(control)
        setNeedsLayout()
      }
    }
  }

  private lazy var control: UIPasteControl = Self.makeControl(labelled: false, target: self)

  /// The key row's look (the Keys board, KeysNormalized): the key's surface ground and text colour
  /// with its corners less the 1 pt rim the view around it draws. A clear ground drew as a black
  /// circle among the row's keys. The glyph stays the system's own.
  private static let keyGround = UIColor(red: 0x1e / 255, green: 0x1e / 255, blue: 0x1e / 255, alpha: 1)
  private static let keyText = UIColor(red: 0xe6 / 255, green: 0xe6 / 255, blue: 0xe6 / 255, alpha: 1)
  private static let keyRadius: CGFloat = 11
  /// palette.ts `bg` and `accent`, for the Attach sheet's Paste row.
  private static let rowGround = UIColor(red: 0x05 / 255, green: 0x05 / 255, blue: 0x05 / 255, alpha: 1)
  private static let accent = UIColor(red: 0x19 / 255, green: 0xaa / 255, blue: 0xd8 / 255, alpha: 1)

  private static func makeControl(labelled: Bool, target: UIView) -> UIPasteControl {
    let config = UIPasteControl.Configuration()
    config.displayMode = labelled ? .iconAndLabel : .iconOnly
    if labelled {
      // the Attach sheet's row: its list's ground and the accent its sibling rows' icons take
      // (a clear ground drew as a black pill there too)
      config.cornerStyle = .large
      config.baseBackgroundColor = rowGround
      config.baseForegroundColor = accent
    } else {
      config.cornerStyle = .fixed
      config.cornerRadius = keyRadius
      config.baseBackgroundColor = keyGround
      config.baseForegroundColor = keyText
    }
    let control = UIPasteControl(configuration: config)
    control.target = target
    return control
  }

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    let accepted = UIPasteConfiguration(acceptableTypeIdentifiers: ["public.image", "public.plain-text", "public.text", "public.data"])
    pasteConfiguration = accepted
    addSubview(control)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    // a key: inside the 1 pt rim the view around it draws, so the rim shows like every key's
    control.frame = labelled ? bounds : bounds.insetBy(dx: 1, dy: 1)
  }

  override func canPaste(_ itemProviders: [NSItemProvider]) -> Bool {
    itemProviders.contains {
      $0.canLoadObject(ofClass: UIImage.self) || $0.canLoadObject(ofClass: NSString.self) || Self.fileType(of: $0) != nil
    }
  }

  override func paste(itemProviders: [NSItemProvider]) {
    guard let item = itemProviders.first else {
      return
    }
    if item.canLoadObject(ofClass: UIImage.self) {
      item.loadObject(ofClass: UIImage.self) { [weak self] object, _ in
        guard let self, let image = object as? UIImage else {
          return
        }
        let png = image.pngData()
        let data = (png?.count ?? Int.max) <= Self.maxPng ? png : image.jpegData(compressionQuality: 0.8)
        guard let data else {
          return
        }
        let b64 = data.base64EncodedString()
        DispatchQueue.main.async { self.onPasteImage(["data": b64]) }
      }
    } else if item.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) || !item.canLoadObject(ofClass: NSString.self) {
      pasteFile(item)
    } else {
      item.loadObject(ofClass: NSString.self) { [weak self] object, _ in
        guard let self, let text = object as? String else {
          return
        }
        DispatchQueue.main.async { self.onPasteText(["text": text]) }
      }
    }
  }

  /// The type a copied file's bytes load as: data, but not a URL's own representation.
  private static func fileType(of item: NSItemProvider) -> String? {
    item.registeredTypeIdentifiers.first { id in
      guard let type = UTType(id) else {
        return false
      }
      return type.conforms(to: .data) && !type.conforms(to: .url)
    }
  }

  /// The bytes are read only within the limit; the temporary copy lasts as long as this closure.
  private func pasteFile(_ item: NSItemProvider) {
    guard let id = Self.fileType(of: item) else {
      onPasteFile(["status": "unreadable"])
      return
    }
    item.loadFileRepresentation(forTypeIdentifier: id) { [weak self] url, _ in
      guard let self else {
        return
      }
      var payload: [String: Any] = ["status": "unreadable"]
      if let url {
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? Int.max
        if size > Self.maxPng {
          payload = ["status": "too_large"]
        } else if let data = try? Data(contentsOf: url) {
          var name = url.lastPathComponent
          if url.pathExtension.isEmpty, let ext = UTType(id)?.preferredFilenameExtension {
            name += ".\(ext)"
          }
          payload = ["status": "ok", "name": name, "data": data.base64EncodedString()]
        }
      }
      DispatchQueue.main.async { self.onPasteFile(payload) }
    }
  }
}
