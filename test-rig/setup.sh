#!/bin/sh
# Builds and configures a local OpenSim standalone for testing RusselBot.
# The simulator lives in test-rig/opensim (git-ignored); the source checkout
# it is cloned from is left untouched. Safe to run again: it only does the
# steps that are still missing, and never overwrites an existing OpenSim.ini.
set -eu

rig=$(cd "$(dirname "$0")" && pwd)
source=${OPENSIM_SRC:-$rig/../references/opensim}
sim=$rig/opensim

if ! dotnet --list-sdks 2>/dev/null | grep -q '^8\.'; then
    echo "The .NET 8 SDK is required to build OpenSim and was not found." >&2
    echo "Install it (Arch/Manjaro: sudo pacman -S dotnet-sdk-8.0) and run this again." >&2
    exit 1
fi

if [ ! -d "$sim" ]; then
    if [ ! -d "$source" ]; then
        echo "No OpenSim source at $source; set OPENSIM_SRC to a checkout." >&2
        exit 1
    fi
    git clone "$source" "$sim"
fi

if [ ! -f "$sim/bin/OpenSim.dll" ]; then
    (cd "$sim" && ./runprebuild.sh && dotnet build --configuration Release OpenSim.sln)
fi

bin=$sim/bin
if [ ! -f "$bin/OpenSim.ini" ]; then
    # Standalone architecture is already the uncommented default in the
    # example; add a default estate so the first start asks no questions.
    sed -e 's|^ *; DefaultEstateName = .*|    DefaultEstateName = Rig Estate|' \
        -e 's|^ *; DefaultEstateOwnerName = .*|    DefaultEstateOwnerName = Rig Owner|' \
        -e 's|^ *; DefaultEstateOwnerUUID = .*|    DefaultEstateOwnerUUID = 7e570000-0000-4000-8000-0000000000aa|' \
        -e 's|^ *; DefaultEstateOwnerEMail = .*|    DefaultEstateOwnerEMail = owner@example.com|' \
        -e 's|^ *; DefaultEstateOwnerPassword = .*|    DefaultEstateOwnerPassword = rigowner|' \
        "$bin/OpenSim.ini.example" > "$bin/OpenSim.ini"
fi
if [ ! -f "$bin/config-include/StandaloneCommon.ini" ]; then
    # Logins that cannot be placed where they ask go to "Home".
    sed -e 's|^ *Region_Welcome_Area = .*|    Region_Home = "DefaultRegion, FallbackRegion"|' \
        "$bin/config-include/StandaloneCommon.ini.example" > "$bin/config-include/StandaloneCommon.ini"
fi
cp "$rig/config/Regions.ini" "$bin/Regions/Regions.ini"

echo "Rig ready. Start it with: (cd $bin && ./opensim.sh)"
