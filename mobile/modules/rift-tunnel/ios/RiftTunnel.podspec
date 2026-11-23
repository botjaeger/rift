Pod::Spec.new do |s|
  s.name           = 'RiftTunnel'
  s.version        = '1.0.0'
  s.summary        = 'A loopback TCP listener whose connections the Rift app forwards to a computer'
  s.description    = 'Listens on 127.0.0.1 and hands each connection to JS as a stream of base64 chunks'
  s.author         = ''
  s.homepage       = 'https://github.com/botjaeger/rift'
  s.license        = 'MIT'
  s.platforms      = { :ios => '16.4' }
  s.swift_version  = '5.9'
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  s.source_files = "**/*.{h,m,mm,swift}"
end
