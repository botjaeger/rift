import ExpoModulesCore

public class RiftPasteModule: Module {
  public func definition() -> ModuleDefinition {
    Name("RiftPaste")

    View(RiftPasteButton.self) {
      Events("onPasteText", "onPasteImage", "onPasteFile")

      Prop("labelled") { (view: RiftPasteButton, labelled: Bool?) in
        view.labelled = labelled ?? false
      }
      Prop("ground") { (view: RiftPasteButton, hex: String?) in
        view.ground = RiftPasteButton.color(hex: hex)
      }
      Prop("ink") { (view: RiftPasteButton, hex: String?) in
        view.ink = RiftPasteButton.color(hex: hex)
      }
    }
  }
}
