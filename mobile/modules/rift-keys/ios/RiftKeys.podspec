Pod::Spec.new do |s|
  s.name           = 'RiftKeys'
  s.version        = '1.0.0'
  s.summary        = 'Hardware and soft-keyboard key capture for the Rift terminal view'
  s.description    = 'A view that takes keys and typed text for the terminal screen, or ctrl/alt shortcuts from a field inside it'
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
