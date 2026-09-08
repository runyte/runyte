#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Runyte experimental-1 example. Python 3, no third-party packages."""
import json
import sys

VERSION = "runyte-experimental-1"


def send(message):
    print(json.dumps(message, ensure_ascii=False), flush=True)


def main():
    hello = json.loads(sys.stdin.readline())
    if hello != {"type": "hello", "version": VERSION}:
        raise SystemExit("unsupported Runyte plugin API")
    send({"type": "register", "version": VERSION, "commands": [
        {"name": "uppercase", "description": "Uppercase every selection"}
    ]})
    for line in sys.stdin:
        message = json.loads(line)
        if message["type"] == "invoke":
            # Python string indices count Unicode scalar values, matching the API.
            text = message["text"]
            send({"type": "replace", "invocation": message["invocation"],
                  "replacements": [text[s["from"]:s["to"]].upper()
                                   for s in message["selections"]]})
        # registered/complete/event messages carry acknowledgements, not work.


if __name__ == "__main__":
    main()
