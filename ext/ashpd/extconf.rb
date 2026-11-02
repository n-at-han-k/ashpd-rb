# frozen_string_literal: true

# Released under the Apache License, Version 2.0.
# Copyright, 2026, by General Intelligence Systems.

require "mkmf"
require "rb_sys/mkmf"

# Builds the Rust crate in this directory and produces ashpd.so.
# The crate's #[magnus::init] defines the classes under Ashpd.
create_rust_makefile("ashpd")
