#!/bin/sh
# Read Cargo's generated lockfile, not an independently maintained CLI pin.
# Exactly one wasm-bindgen schema must be selected across the workspace.
set -eu
awk -F '"' '
    function finish() {
        if (name == "wasm-bindgen") {
            count++
            selected = version
        }
        name = version = ""
    }
    /^\[/ {
        finish()
        package = ($0 == "[[package]]")
    }
    package && /^name[[:space:]]*=/ { name = $2 }
    package && /^version[[:space:]]*=/ { version = $2 }
    END {
        finish()
        if (count != 1 || selected !~ /^[0-9]+\.[0-9]+\.[0-9]+$/) {
            print "Expected exactly one locked wasm-bindgen release version" > "/dev/stderr"
            exit 1
        }
        print selected
    }
' "${1:-Cargo.lock}"
