"""pytest plugin (`-p rs_shim`): upstream Calliope tests read their models with calliope_rs.

Only `calliope.read_yaml` is replaced, so tests that build their models with it run on
calliope-rs; everything else stays Python Calliope.
"""

import calliope

import calliope_rs

calliope.read_yaml = calliope_rs.read_yaml
