import ExpoModulesCore
import Foundation
import Network

/**
 * A TCP listener on this phone's loopback (127.0.0.1 only): every connection to it becomes a
 * stream whose bytes JS reads (`onData`, base64) and writes (`write`). JS forwards them to the
 * computer, so a page in the browser sheet can load the computer's localhost.
 */
public class RiftTunnelModule: Module {
  private let queue = DispatchQueue(label: "dev.botjaeger.rift.tunnel")
  private var listener: NWListener?
  private var streams: [Int: NWConnection] = [:]
  private var nextStream = 1

  public func definition() -> ModuleDefinition {
    Name("RiftTunnel")

    Events("onOpen", "onData", "onEnd", "onClose")

    AsyncFunction("listen") { (port: Int, promise: Promise) in
      self.queue.async { self.listen(port: port, fallback: true, promise: promise) }
    }

    AsyncFunction("stop") { (promise: Promise) in
      self.queue.async {
        self.stopAll()
        promise.resolve(nil)
      }
    }

    AsyncFunction("write") { (stream: Int, base64: String, promise: Promise) in
      self.queue.async {
        guard let conn = self.streams[stream] else {
          promise.resolve(false)
          return
        }
        guard let data = Data(base64Encoded: base64) else {
          promise.reject("ERR_BASE64", "write: not base64")
          return
        }
        // resolved once sent: JS acknowledges the bytes to the computer then, so a browser that
        // reads slowly holds the computer's window, not a growing queue here
        conn.send(content: data, completion: .contentProcessed { [weak self] error in
          if error != nil { self?.queue.async { self?.finish(stream) } }
          promise.resolve(error == nil)
        })
      }
    }

    AsyncFunction("close") { (stream: Int, promise: Promise) in
      self.queue.async {
        self.finish(stream)
        promise.resolve(nil)
      }
    }

    OnDestroy {
      self.queue.sync { self.stopAll() }
    }
  }

  /// Binds 127.0.0.1:`port`; when it is taken (or 0), an ephemeral port. Resolves with the bound port.
  private func listen(port: Int, fallback: Bool, promise: Promise) {
    stopAll()
    let wanted: NWEndpoint.Port = (port > 0 && port <= 65535) ? NWEndpoint.Port(rawValue: UInt16(port)) ?? .any : .any
    let params = NWParameters.tcp
    params.acceptLocalOnly = true
    params.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: wanted)
    let l: NWListener
    do {
      l = try NWListener(using: params)
    } catch {
      if fallback && wanted != .any {
        listen(port: 0, fallback: false, promise: promise)
      } else {
        promise.reject("ERR_LISTEN", "listen: \(error)")
      }
      return
    }
    var settled = false
    l.stateUpdateHandler = { [weak self] state in
      guard let self = self else { return }
      switch state {
      case .ready:
        if !settled {
          settled = true
          let bound = Int(l.port?.rawValue ?? 0)
          NSLog("RiftTunnel: listening on 127.0.0.1:%d", bound)
          promise.resolve(bound)
        }
      case .failed(let error), .waiting(let error):
        l.cancel()
        if self.listener === l { self.listener = nil }
        if !settled {
          settled = true
          NSLog("RiftTunnel: bind %d failed: %@", Int(wanted.rawValue), "\(error)")
          if fallback && wanted != .any {
            self.listen(port: 0, fallback: false, promise: promise)
          } else {
            promise.reject("ERR_LISTEN", "listen: \(error)")
          }
        }
      default:
        break
      }
    }
    l.newConnectionHandler = { [weak self] conn in self?.accept(conn) }
    listener = l
    l.start(queue: queue)
  }

  private func accept(_ conn: NWConnection) {
    let id = nextStream
    nextStream += 1
    streams[id] = conn
    conn.stateUpdateHandler = { [weak self] state in
      switch state {
      case .failed, .cancelled:
        self?.finish(id)
      default:
        break
      }
    }
    conn.start(queue: queue)
    NSLog("RiftTunnel: open stream %d from %@", id, "\(conn.endpoint)")
    sendEvent("onOpen", ["stream": id])
    receive(id, conn)
  }

  private func receive(_ id: Int, _ conn: NWConnection) {
    conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
      guard let self = self else { return }
      if let data = data, !data.isEmpty {
        NSLog("RiftTunnel: stream %d read %d bytes", id, data.count)
        self.sendEvent("onData", ["stream": id, "data": data.base64EncodedString()])
      }
      if error != nil {
        self.finish(id)
      } else if isComplete {
        // The peer finished sending (half-close): the stream stays writable until close().
        if self.streams[id] != nil {
          NSLog("RiftTunnel: stream %d end", id)
          self.sendEvent("onEnd", ["stream": id])
        }
      } else {
        self.receive(id, conn)
      }
    }
  }

  /// Ends a stream once: its queued writes go out first, then the connection closes.
  private func finish(_ id: Int) {
    guard let conn = streams.removeValue(forKey: id) else { return }
    conn.send(content: nil, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed { _ in
      conn.cancel()
    })
    NSLog("RiftTunnel: close stream %d", id)
    sendEvent("onClose", ["stream": id])
  }

  private func stopAll() {
    if let l = listener {
      NSLog("RiftTunnel: stopped listening on 127.0.0.1:%d", Int(l.port?.rawValue ?? 0))
    }
    listener?.cancel()
    listener = nil
    for id in Array(streams.keys) { finish(id) }
  }
}
