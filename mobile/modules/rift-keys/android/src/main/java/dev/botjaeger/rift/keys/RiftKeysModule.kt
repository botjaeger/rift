package dev.botjaeger.rift.keys

import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class RiftKeysModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("RiftKeys")

    View(KeyCaptureView::class) {
      Events("onText", "onKey", "onFocusChange")

      Prop("mode") { view: KeyCaptureView, mode: String ->
        view.setMode(mode == "terminal")
      }

      Prop("captureTab") { view: KeyCaptureView, capture: Boolean ->
        view.captureTab = capture
      }

      AsyncFunction("focus") { view: KeyCaptureView ->
        view.focusCapture()
      }.runOnQueue(Queues.MAIN)

      AsyncFunction("blur") { view: KeyCaptureView ->
        view.blurCapture()
      }.runOnQueue(Queues.MAIN)
    }
  }
}
