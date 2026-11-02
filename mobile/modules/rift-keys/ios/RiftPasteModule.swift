import ExpoModulesCore

public class RiftPasteModule: Module {
  public func definition() -> ModuleDefinition {
    Name("RiftPaste")

    View(RiftPasteButton.self) {
      Events("onPasteText", "onPasteImage", "onPasteFile")

      Prop("labelled") { (view: RiftPasteButton, labelled: Bool?) in
        view.labelled = labelled ?? false
      }
    }
  }
}
