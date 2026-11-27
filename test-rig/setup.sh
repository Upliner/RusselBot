#!/bin/sh
# Builds and configures a local OpenSim standalone for testing RusselBot.
# The simulator lives in test-rig/opensim (git-ignored); the source checkout
# it is cloned from is left untouched. Safe to run again: it only does the
# steps that are still missing, and never overwrites an existing OpenSim.ini.
set -eu

rig=$(cd "$(dirname "$0")" && pwd)
source=${OPENSIM_SRC:-$rig/../references/opensim}
sim=$rig/opensim

# Prosody for the XMPP gateway, when it is installed: a config in
# test-rig/prosody (git-ignored) with the accounts owner@rig.test and
# stranger@rig.test and the component bot.rig.test.
xmpp=$rig/prosody
if command -v prosodyctl >/dev/null 2>&1; then
    mkdir -p "$xmpp/data" "$xmpp/certs"
    sed -e "s|@RIG_PROSODY@|$xmpp|g" "$rig/config/prosody.cfg.lua.in" > "$xmpp/prosody.cfg.lua"
    # TLS for clients that insist on it (Dino): a CA of the rig's own and a
    # certificate for rig.test signed by it. Trust rig-ca.crt to use them.
    certs=$xmpp/certs
    if [ ! -f "$certs/rig.test.crt" ] && command -v openssl >/dev/null 2>&1; then
        openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=RusselBot test rig CA" \
            -keyout "$certs/rig-ca.key" -out "$certs/rig-ca.crt" 2>/dev/null
        openssl req -newkey rsa:2048 -nodes -subj "/CN=rig.test" \
            -keyout "$certs/rig.test.key" -out "$certs/rig.test.csr" 2>/dev/null
        printf 'subjectAltName=DNS:rig.test\nextendedKeyUsage=serverAuth\n' > "$certs/rig.test.ext"
        openssl x509 -req -in "$certs/rig.test.csr" -CA "$certs/rig-ca.crt" -CAkey "$certs/rig-ca.key" \
            -CAcreateserial -days 3650 -extfile "$certs/rig.test.ext" -out "$certs/rig.test.crt" 2>/dev/null
        rm -f "$certs/rig.test.csr" "$certs/rig.test.ext"
    fi
    for account in owner stranger; do
        if [ ! -d "$xmpp/data/rig%2etest/accounts" ] || [ ! -f "$xmpp/data/rig%2etest/accounts/$account.dat" ]; then
            prosodyctl --config "$xmpp/prosody.cfg.lua" register "$account" rig.test "$account" >/dev/null
        fi
    done
    echo "Prosody ready. Start it with: prosody --config $xmpp/prosody.cfg.lua -F"
else
    echo "Prosody is not installed; skipping the XMPP part of the rig." >&2
fi

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

# The rig's money module (test-rig/money) and its in-memory store for
# Groups V2 (test-rig/groups) are built with the simulator as addon
# modules. A copy that changed is copied again, which makes it newer than
# the built dll and so rebuilds the simulator with it.
build=no
for module in money:RigMoney:RigMoneyModule groups:RigGroups:RigGroupsData; do
    from=${module%%:*}; rest=${module#*:}; dir=${rest%%:*}; name=${rest#*:}
    addon=$sim/addon-modules/$dir
    mkdir -p "$addon"
    for file in "$rig/$from"/*.cs "$rig/$from/prebuild.xml"; do
        copy=$addon/$(basename "$file")
        cmp -s "$file" "$copy" || cp "$file" "$copy"
    done
    dll=$sim/bin/$name.dll
    if [ ! -f "$dll" ] || [ -n "$(find "$addon" -name '*.cs' -newer "$dll" -o -name prebuild.xml -newer "$dll")" ]; then
        build=yes
    fi
done
if [ ! -f "$sim/bin/OpenSim.dll" ] || [ "$build" = yes ]; then
    (cd "$sim" && ./runprebuild.sh && dotnet build --configuration Release OpenSim.sln)
fi

bin=$sim/bin
if [ ! -f "$bin/OpenSim.ini" ]; then
    # Standalone architecture is already the uncommented default in the
    # example; add a default estate so the first start asks no questions,
    # the profile service, without which nothing answers for profiles, and
    # the rig's money module.
    sed -e 's|^ *; DefaultEstateName = .*|    DefaultEstateName = Rig Estate|' \
        -e 's|^ *;; ProfileServiceURL = |  ProfileServiceURL = |' \
        -e 's|^ *; DefaultEstateOwnerName = .*|    DefaultEstateOwnerName = Rig Owner|' \
        -e 's|^ *; DefaultEstateOwnerUUID = .*|    DefaultEstateOwnerUUID = 7e570000-0000-4000-8000-0000000000aa|' \
        -e 's|^ *; DefaultEstateOwnerEMail = .*|    DefaultEstateOwnerEMail = owner@example.com|' \
        -e 's|^ *; DefaultEstateOwnerPassword = .*|    DefaultEstateOwnerPassword = rigowner|' \
        -e 's|^ *; economymodule = BetaGridLikeMoneyModule|    economymodule = RigMoneyModule|' \
        "$bin/OpenSim.ini.example" > "$bin/OpenSim.ini"
    # Groups V2 on the rig's store, with chat and notices. The groups
    # service looks for the store of [GridUserService] there too, unless
    # that section names its own.
    sed -i -e '/^\[Groups\]/a\
    Enabled = true\
    Module = "Groups Module V2"\
    StorageProvider = "RigGroupsData.dll"\
    ServicesConnectorModule = "Groups Local Service Connector"\
    LocalService = local\
    MessagingEnabled = true\
    MessagingModule = "Groups Messaging Module V2"\
    MessageOnlineUsersOnly = true\
    NoticesEnabled = true' "$bin/OpenSim.ini"
    printf '\n[GridUserService]\n    StorageProvider = "OpenSim.Data.SQLite.dll"\n' >> "$bin/OpenSim.ini"
fi
if [ ! -f "$bin/config-include/StandaloneCommon.ini" ]; then
    # Logins that cannot be placed where they ask go to "Home"; the local
    # profile service is on (the first `Enabled = false` is its section's).
    sed -e 's|^ *Region_Welcome_Area = .*|    Region_Home = "DefaultRegion, FallbackRegion"|' \
        -e '/^\[UserProfilesService\]/,/^ *Enabled/s|Enabled = false|Enabled = true|' \
        "$bin/config-include/StandaloneCommon.ini.example" > "$bin/config-include/StandaloneCommon.ini"
fi
cp "$rig/config/Regions.ini" "$bin/Regions/Regions.ini"

echo "Rig ready. Start it with: (cd $bin && ./opensim.sh)"
