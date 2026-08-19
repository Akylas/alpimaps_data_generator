#!/usr/bin/env bash
# Size ladder + per-layer breakdown for every bench/*.mbtiles built by run.sh
cd "$(dirname "$0")"

echo "=== file sizes ==="
base=$(stat -f%z base.mbtiles)
for f in base v2_nameint v3_simplify14 v4_simplify_all v5_no_housenumber v6_no_building; do
  [ -f "$f.mbtiles" ] || continue
  s=$(stat -f%z "$f.mbtiles")
  awk -v n="$f" -v s="$s" -v b="$base" 'BEGIN{printf "%-20s %8.1f MB   %+7.1f%% vs base\n", n, s/1048576, 100*(s-b)/b}'
done

echo
echo "=== per-layer uncompressed bytes (MB) ==="
for f in base v2_nameint v3_simplify14 v4_simplify_all v5_no_housenumber v6_no_building; do
  [ -f "$f.mbtiles.layerstats.tsv.gz" ] || continue
  gzcat "$f.mbtiles.layerstats.tsv.gz" | awk -F'\t' -v n="$f" 'NR>1{b[$6]+=$7} END{for(k in b) printf "%s\t%s\t%.2f\n", k, n, b[k]/1048576}'
done | sort | awk '{d[$1]=d[$1] sprintf("%10.1f",$3); o[$1]=1} END{for(k in o) printf "%-20s%s\n", k, d[k]}' | sort
