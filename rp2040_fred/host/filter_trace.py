#!/usr/bin/env python3
"""
filter_trace.py - Filter bus trace log lines.

Elides lines matching the pattern:
    (03|02|01|00|07|06|05|04|0D|0C)  80   W
AND also elides the *following* line if that line contains 'R'.

Usage:
    python filter_trace.py input.txt              # prints to stdout
    python filter_trace.py input.txt -o out.txt   # writes to file
    cat input.txt | python filter_trace.py        # reads from stdin
"""

import re
import sys
import argparse

# Matches the address-byte pattern for 80-bus writes we want to elide.
# Looks for the A-column value (hex byte) followed by "  80   W" anywhere in the line.
ELIDE_PATTERN = re.compile(r'\b(03|02|01|00|07|06|05|04|0D|0C)\s+80\s+W\b')

# Matches the batch_us column: a run of digits that appears as the 3rd whitespace-
# separated field on data lines.  We capture the surrounding whitespace so we can
# replace it with a same-width seconds string and keep column alignment intact.
BATCH_US_RE = re.compile(
    r'^(\s*\S+\s+\S+\s+)'   # group 1: step + delta_us (fields 1-2)
    r'(\d+)'                  # group 2: batch_us value
    r'(\s+)',                 # group 3: whitespace after batch_us
    re.MULTILINE,
)


def convert_batch_us(line):
    """
    Replace the batch_us field with a seconds value (same column width).
    Header lines (containing 'batch_us') get the column renamed to 'batch_s'.
    Non-matching lines are returned unchanged.
    """
    # Rename the header column
    if 'batch_us' in line:
        return line.replace('batch_us', ' batch_s', 1)

    m = BATCH_US_RE.match(line)
    if not m:
        return line

    batch_us = int(m.group(2))
    batch_s = batch_us / 1_000_000

    # Format to the same character width as the original integer, with enough
    # decimal places to preserve microsecond resolution (6 dp).
    original_width = len(m.group(2))
    # e.g. 745811542 (9 chars) -> "745.811542" (10 chars) — allow one extra char
    formatted = f'{batch_s:.6f}'
    # Pad to at least the original width so downstream columns don't shift much
    if len(formatted) < original_width:
        formatted = formatted.rjust(original_width)

    return m.group(1) + formatted + m.group(3) + line[m.end():]


def filter_lines(lines):
    """Yield lines that should be kept, applying the elision rules."""
    skip_next_if_read = False

    for line in lines:
        if skip_next_if_read:
            skip_next_if_read = False
            # Drop this line only if it contains a Read ('R' in the RnW column).
            # We check for ' R ' or ' R\n' / end-of-string to avoid false matches.
            if re.search(r'\bR\b', line):
                continue  # elide it
            else:
                yield convert_batch_us(line)
                continue

        if ELIDE_PATTERN.search(line):
            skip_next_if_read = True  # conditionally elide the next line
            continue  # elide this line

        yield convert_batch_us(line)


def main():
    parser = argparse.ArgumentParser(description="Filter bus-trace log lines.")
    parser.add_argument(
        "input", nargs="?", default="-",
        help="Input file path (default: stdin)"
    )
    parser.add_argument(
        "-o", "--output", default="-",
        help="Output file path (default: stdout)"
    )
    args = parser.parse_args()

    # --- open input ---
    if args.input == "-":
        in_fh = sys.stdin
    else:
        in_fh = open(args.input, "r", encoding="utf-8")

    # --- open output ---
    if args.output == "-":
        out_fh = sys.stdout
    else:
        out_fh = open(args.output, "w", encoding="utf-8")

    try:
        lines = in_fh.readlines()
        for line in filter_lines(lines):
            out_fh.write(line)
    finally:
        if in_fh is not sys.stdin:
            in_fh.close()
        if out_fh is not sys.stdout:
            out_fh.close()


if __name__ == "__main__":
    main()
