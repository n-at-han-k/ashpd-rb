# frozen_string_literal: true

# Released under the Apache License, Version 2.0.
# Copyright, 2026, by General Intelligence Systems.

require_relative "ashpd/version"

# The native extension, which is where every class in this module is defined.
#
# A precompiled platform gem carries one object per Ruby ABI, which rake-compiler puts under lib/ashpd/3.3, 3.4 and so on. The source gem compiles one object and installs it beside the library, which is also where `rake compile` puts it.
begin
  require_relative "ashpd/#{RUBY_VERSION[/\d+\.\d+/]}/ashpd"
rescue LoadError
  require_relative "ashpd/ashpd"
end

# Ruby bindings for ashpd, the Rust XDG Desktop Portal client.
#
# The portals are how a sandboxed application asks the user for something it cannot take: a screenshot, a file outside its sandbox, the camera, the trash. ashpd speaks the `org.freedesktop.portal.*` D-Bus interfaces through zbus and does not load libportal or libxdp, so neither is in this process.
#
# Each portal is a module named for its interface: {Screenshot}, {OpenUri}, {Trash}, {Settings}, {Camera}, {Account}.
#
# Every call is synchronous, and a portal call that shows a dialog does not return until the user answers it. On a GTK main loop that freezes the window the dialog is modal to, so put these on a Ruby thread.
module Ashpd
end
