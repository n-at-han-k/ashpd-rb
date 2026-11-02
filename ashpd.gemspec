# frozen_string_literal: true

# Released under the Apache License, Version 2.0.
# Copyright, 2026, by General Intelligence Systems.

require_relative "lib/ashpd/version"

Gem::Specification.new do |spec|
  spec.name = "ashpd"
  spec.version = Ashpd::VERSION
  spec.authors = ["Nathan Kidd"]
  spec.email = ["nathankidd@hey.com"]
  spec.license = "Apache-2.0"

  spec.summary = "Ruby bindings for ashpd, the Rust XDG Desktop Portal client."
  spec.description = "The XDG desktop portals for Ruby, over the ashpd crate. Screenshots, file "\
    "chooser, trash, settings, camera and account -- speaking the portal D-Bus interfaces "\
    "directly through zbus, so neither libportal nor libxdp is in the picture."
  spec.homepage = "https://github.com/n-at-han-k/ashpd-rb"

  spec.required_ruby_version = ">= 3.3"

  spec.metadata["source_code_uri"] = spec.homepage
  spec.metadata["documentation_uri"] = "https://n-at-han-k.github.io/ashpd-rb/"

  # The library, and the native extension's sources.
  #
  # The crate ships because spec.extensions builds it at install time from the source gem, and Cargo.toml and Cargo.lock ship with it because they are the workspace root rb_sys runs `cargo metadata` from. A precompiled platform gem carries the built object instead, and builds nothing.
  #
  # The built object is rejected: `rake compile` installs one into lib/ashpd, and that one belongs to this machine's Ruby. The source gem compiles its own, and rake-compiler puts the right one into each precompiled gem itself.
  #
  # LICENSE and NOTICE ship because the Apache License asks for both: section 4(a) wants every recipient given a copy of the licence, and 4(d) wants the attribution notices carried along with it.
  spec.files = Dir.glob(
    ["{lib}/**/*", "{ext}/**/*.{rs,rb,toml}", "Cargo.toml", "Cargo.lock", "LICENSE", "NOTICE", "README.md", "CHANGELOG.md"],
    File::FNM_DOTMATCH,
    base: __dir__,
  ).reject{|path| File.fnmatch?("*.{so,bundle,dll,o,a}", path, File::FNM_EXTGLOB)}

  spec.require_paths = ["lib"]
  spec.extensions = ["ext/ashpd/extconf.rb"]

  # GemKit::Deprecate declares a deprecation as a dated promise: the replacement, and the version the old name stops existing in.
  spec.add_dependency "gem_kit", "~> 0.2"

  # Wanted at install time rather than only in development: ext/ashpd/extconf.rb requires rb_sys/mkmf, so installing the source gem resolves this before it can compile.
  spec.add_dependency "rb_sys", "~> 0.9"

  # The Rakefile's RbSys::ExtensionTask, which `rake compile` and the cross-compile workflow drive.
  spec.add_development_dependency "rake-compiler", "~> 1.3"
  spec.add_development_dependency "lefthook", "~> 2.2"
end
