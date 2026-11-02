# frozen_string_literal: true

# Released under the Apache License, Version 2.0.
# Copyright, 2026, by General Intelligence Systems.

require "ashpd"

describe Ashpd do
  it "defines every module the crate's init names" do
    required = [:Screenshot, :OpenUri, :Trash, :Settings, :Camera, :Account]

    expect(required - Ashpd.constants).to be(:empty?)
  end

  it "defines the method on each portal module" do
    expect(Ashpd::Screenshot).to be(:respond_to?, :capture)
    expect(Ashpd::Screenshot).to be(:respond_to?, :pick_color)
    expect(Ashpd::OpenUri).to be(:respond_to?, :open)
    expect(Ashpd::Trash).to be(:respond_to?, :file)
    expect(Ashpd::Settings).to be(:respond_to?, :color_scheme)
    expect(Ashpd::Settings).to be(:respond_to?, :accent_color)
    expect(Ashpd::Camera).to be(:respond_to?, :present?)
    expect(Ashpd::Account).to be(:respond_to?, :user_information)
  end

  # Everything below asserts against the portal's absence rather than its
  # presence. There is no xdg-desktop-portal on the session bus in the build
  # sandbox or in CI, and a test that needed one would be a test that never
  # runs. What is worth pinning here is that a missing portal surfaces as a
  # Ruby exception carrying the D-Bus error, and not as a panic across the FFI
  # boundary -- a panic in the extension aborts the VM, so the difference is
  # the whole safety property of the binding.
  #
  # On a desktop with a portal running these raise nothing and return real
  # values; the ones that would open a dialog are deliberately not called.
  it "raises rather than panics when no portal answers" do
    expect { Ashpd::Settings.color_scheme }.to raise_exception(StandardError)
  end

  it "reports a missing camera portal as an error, not false" do
    expect { Ashpd::Camera.present? }.to raise_exception(StandardError)
  end

  it "refuses a URI the portal could never open" do
    expect { Ashpd::OpenUri.open("not a uri", false, false) }.to raise_exception(StandardError)
  end

  it "refuses to trash a path that does not exist" do
    expect { Ashpd::Trash.file("/nonexistent/#{Process.pid}") }.to raise_exception(StandardError)
  end
end
