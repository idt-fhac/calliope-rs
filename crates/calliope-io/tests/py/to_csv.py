"""Reference for the Rust CSV export: `calliope.read_netcdf(NC).to_csv(OUT_DIR)`."""

import sys

import calliope

calliope.read_netcdf(sys.argv[1]).to_csv(sys.argv[2])
