# frozen_string_literal: true

# Released under the Apache License, Version 2.0.
# Copyright, 2026, by General Intelligence Systems.

require "ashpd"

describe Ashpd::ScreenCast do
  it "defines the portal's queries" do
    expect(Ashpd::ScreenCast).to be(:respond_to?, :available_source_types)
    expect(Ashpd::ScreenCast).to be(:respond_to?, :available_cursor_modes)
    expect(Ashpd::ScreenCast).to be(:respond_to?, :version)
  end

  it "defines a session class, because a cast is not one call" do
    expect(Ashpd::ScreenCast::Session).to be(:respond_to?, :new)

    [:select_sources, :start, :restore_token, :open_pipe_wire_remote, :close].each do |name|
      expect(Ashpd::ScreenCast::Session.instance_methods).to be(:include?, name)
    end
  end

  # As in the suite beside this one, what is asserted is the absence of a
  # panic rather than any particular answer. There is no portal on the session
  # bus in CI, and on a desktop there is one, so the honest assertion is that
  # either outcome arrives as ordinary Ruby: a session object, or an exception
  # a caller can rescue. A panic would take the VM with it instead.
  it "either creates a session or raises, and does not abort the VM" do
    session = begin
      Ashpd::ScreenCast::Session.new
    rescue StandardError
      nil
    end

    expect(session.nil? || session.is_a?(Ashpd::ScreenCast::Session)).to be == true
  end

  it "refuses a source type the specification does not define" do
    session = begin
      Ashpd::ScreenCast::Session.new
    rescue StandardError
      nil
    end

    # Without a portal there is no session to select sources on, and the
    # validation this pins is only reachable through one.
    if session
      expect do
        session.select_sources(sources: [:elephant])
      end.to raise_exception(StandardError)

      session.close
    end
  end
end
