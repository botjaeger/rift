Pod::Spec.new do |s|
  s.name           = 'RiftLiveActivity'
  s.version        = '1.0.0'
  s.summary        = 'Live Activities for commands running on a Rift computer'
  s.description    = 'Starts, updates and ends the lock-screen Live Activity of a running command'
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
