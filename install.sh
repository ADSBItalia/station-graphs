#!/usr/bin/env bash
# ==============================================================================
# ADSBItalia Station Graphs - 1-Click Automated Installer
# https://github.com/ADSBItalia/station-graphs
# ==============================================================================
set -euo pipefail

INSTALL_DIR="/opt/adsbitalia-station-graphs"
SERVICE_NAME="adsbitalia-station-graphs"
REPO_URL="https://github.com/ADSBItalia/station-graphs.git"
DEFAULT_PORT="8085"

info() { echo -e "\e[34m[INFO]\e[0m $*"; }
success() { echo -e "\e[32m[SUCCESS]\e[0m $*"; }
warn() { echo -e "\e[33m[WARN]\e[0m $*"; }
error() { echo -e "\e[31m[ERROR]\e[0m $*" >&2; exit 1; }

# Root check
if [[ $EUID -ne 0 ]]; then
  if command -v sudo >/dev/null 2>&1; then
    exec sudo bash "$0" "$@"
  else
    error "This installer must be run as root or with sudo."
  fi
fi

echo "=================================================================="
echo "    ADSBItalia Station Telemetry & RF Performance Graphs          "
echo "    Modern Avionics Telemetry Engine for ADS-B / MLAT Stations    "
echo "=================================================================="

INSTALL_RADAR=false
if [ -t 0 ]; then
  echo ""
  read -r -p "Do you want to proceed with installing ADSBItalia Station Graphs? [Y/n] " ans_graphs
  if [[ "$ans_graphs" =~ ^[Nn] ]]; then
    info "Installation cancelled by user."
    exit 0
  fi

  read -r -p "Do you also want to install ADSBItalia Station Radar (Local 3D Avionics Map)? [y/N] " ans_radar
  if [[ "$ans_radar" =~ ^[Yy] ]]; then
    INSTALL_RADAR=true
  fi
  echo ""
fi

# Check and install system dependencies
info "Checking system dependencies..."
if command -v apt-get >/dev/null 2>&1; then
  MISSING_PKGS=()
  for pkg in curl git rrdtool; do
    if ! command -v "$pkg" >/dev/null 2>&1; then
      MISSING_PKGS+=("$pkg")
    fi
  done
  if [[ ${#MISSING_PKGS[@]} -gt 0 ]]; then
    info "Installing missing dependencies: ${MISSING_PKGS[*]}..."
    apt-get update -qq || true
    apt-get install -y --no-install-recommends "${MISSING_PKGS[@]}" || true
  fi
fi

# Create directory structure
info "Preparing installation directory at ${INSTALL_DIR}..."
mkdir -p "${INSTALL_DIR}/web"

# Download or update files
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

info "Fetching latest station-graphs release..."
git clone --depth 1 "${REPO_URL}" "${TMP_DIR}/repo" 2>/dev/null || {
  info "Cloning failed, trying archive download..."
  mkdir -p "${TMP_DIR}/repo"
  curl -sL https://github.com/ADSBItalia/station-graphs/archive/refs/heads/main.tar.gz | tar -xz --strip-components=1 -C "${TMP_DIR}/repo"
}

# Copy web files
cp -r "${TMP_DIR}/repo/web/"* "${INSTALL_DIR}/web/"

# Binary build or install
if [[ -f "${TMP_DIR}/repo/adsbitalia-station-graphs" ]]; then
  info "Installing pre-compiled binary..."
  cp "${TMP_DIR}/repo/adsbitalia-station-graphs" "${INSTALL_DIR}/adsbitalia-station-graphs"
  chmod +x "${INSTALL_DIR}/adsbitalia-station-graphs"
elif command -v cargo >/dev/null 2>&1; then
  info "Building Rust telemetry binary from source with cargo..."
  (
    cd "${TMP_DIR}/repo"
    cargo build --release
    cp "target/release/adsbitalia-station-graphs" "${INSTALL_DIR}/adsbitalia-station-graphs"
    chmod +x "${INSTALL_DIR}/adsbitalia-station-graphs"
  )
else
  info "Rust toolchain not found. Installing minimal rustc & cargo..."
  if command -v apt-get >/dev/null 2>&1; then
    apt-get install -y --no-install-recommends cargo rustc || {
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
      export PATH="$HOME/.cargo/bin:$PATH"
    }
    (
      cd "${TMP_DIR}/repo"
      cargo build --release
      cp "target/release/adsbitalia-station-graphs" "${INSTALL_DIR}/adsbitalia-station-graphs"
      chmod +x "${INSTALL_DIR}/adsbitalia-station-graphs"
    )
  else
    error "Cannot build binary: please install rust & cargo."
  fi
fi

# Detect station name and coordinates from local system if available
STATION_NAME="ADSB Station"
STATION_LAT=""
STATION_LON=""

if [[ -f /etc/adsbitalia/feeder.conf ]]; then
  # Source feeder.conf if present
  # shellcheck source=/dev/null
  source /etc/adsbitalia/feeder.conf || true
  if [[ -n "${STATION_NAME:-}" ]]; then
    STATION_NAME="${STATION_NAME}"
  elif [[ -n "${FEEDER_NAME:-}" ]]; then
    STATION_NAME="${FEEDER_NAME}"
  fi
fi

if [[ -f /opt/adsbitalia-station-radar/config.json ]]; then
  # Read name from station-radar config
  RADAR_NAME=$(grep -o '"name": *"[^"]*"' /opt/adsbitalia-station-radar/config.json | head -n1 | cut -d'"' -f4 || true)
  if [[ -n "${RADAR_NAME}" ]]; then
    STATION_NAME="${RADAR_NAME}"
  fi
fi

# Detect Ultrafeeder or local ADS-B source
AIRCRAFT_SOURCE=""
if [ -f "/run/readsb/aircraft.json" ]; then
  AIRCRAFT_SOURCE="/run/readsb/aircraft.json"
elif [ -f "/run/dump1090-fa/aircraft.json" ]; then
  AIRCRAFT_SOURCE="/run/dump1090-fa/aircraft.json"
elif command -v docker >/dev/null 2>&1 && docker ps 2>/dev/null | grep -qi "ultrafeeder"; then
  UF_CID=$(docker ps -q --filter "name=ultrafeeder" | head -n1)
  UF_PORT=$(docker port "$UF_CID" 80 2>/dev/null | awk -F: '{print $NF}' | head -n1 || echo "")
  [ -z "$UF_PORT" ] && UF_PORT=$(docker port "$UF_CID" 8080 2>/dev/null | awk -F: '{print $NF}' | head -n1 || echo "")
  [ -z "$UF_PORT" ] && UF_PORT="8080"
  AIRCRAFT_SOURCE="http://127.0.0.1:${UF_PORT}/data/aircraft.json"
  info "Detected Ultrafeeder container! Connecting live stream to ${AIRCRAFT_SOURCE}"
else
  for probe_url in "http://127.0.0.1:8080/data/aircraft.json" "http://127.0.0.1/data/aircraft.json"; do
    if curl -s -m 2 "$probe_url" 2>/dev/null | grep -q "aircraft"; then
      AIRCRAFT_SOURCE="$probe_url"
      info "Detected active ADS-B stream at ${AIRCRAFT_SOURCE}"
      break
    fi
  done
fi

# Detect collectd RRD directory
RRD_SOURCE=""
for cand in \
  "/run/collectd/localhost/dump1090-localhost" \
  "/var/lib/collectd/rrd/localhost/dump1090-localhost" \
  "/opt/adsb/ultrafeeder/graphs1090/rrd/localhost/dump1090-localhost" \
  "/opt/adsb/ultrafeeder/graphs1090/localhost/dump1090-localhost" \
  "/var/lib/docker/volumes/ultrafeeder_graphs1090/_data/rrd/localhost/dump1090-localhost"; do
  if [ -d "$cand" ]; then
    RRD_SOURCE="$(dirname "$cand")"
    info "Detected collectd RRD metrics directory at ${RRD_SOURCE}"
    break
  fi
done

# Write config if not present
if [[ ! -f "${INSTALL_DIR}/config.json" ]]; then
  if [[ -n "${AIRCRAFT_SOURCE}" ]] && [[ -n "${RRD_SOURCE}" ]]; then
    cat > "${INSTALL_DIR}/config.json" <<EOF
{
  "station_name": "${STATION_NAME}",
  "port": ${DEFAULT_PORT},
  "aircraft_json": "${AIRCRAFT_SOURCE}",
  "dump_rrd": "${RRD_SOURCE}/dump1090-localhost",
  "cpu_rrd": "${RRD_SOURCE}/aggregation-cpu-average",
  "temp_rrd": "${RRD_SOURCE}/table-localhost"
}
EOF
  elif [[ -n "${AIRCRAFT_SOURCE}" ]]; then
    cat > "${INSTALL_DIR}/config.json" <<EOF
{
  "station_name": "${STATION_NAME}",
  "port": ${DEFAULT_PORT},
  "aircraft_json": "${AIRCRAFT_SOURCE}"
}
EOF
  else
    cat > "${INSTALL_DIR}/config.json" <<EOF
{
  "station_name": "${STATION_NAME}",
  "port": ${DEFAULT_PORT}
}
EOF
  fi
fi

# Create systemd service
info "Installing systemd service (${SERVICE_NAME}.service)..."
cat > "/etc/systemd/system/${SERVICE_NAME}.service" <<EOF
[Unit]
Description=ADSBItalia Station Telemetry & RF Performance Graphs
After=network.target readsb.service dump1090-fa.service

[Service]
Type=simple
User=root
WorkingDirectory=${INSTALL_DIR}
ExecStart=${INSTALL_DIR}/adsbitalia-station-graphs
Restart=always
RestartSec=3
LimitNOFILE=65535
Environment=PORT=${DEFAULT_PORT}
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "${SERVICE_NAME}"
systemctl restart "${SERVICE_NAME}"

# Detect LAN IP
LAN_IP="127.0.0.1"
if command -v hostname >/dev/null 2>&1; then
  LAN_IP=$(hostname -I 2>/dev/null | awk '{print $1}' || echo "127.0.0.1")
fi

echo ""
echo "=================================================================="
success "ADSBItalia Station Graphs successfully installed and running!"
echo "=================================================================="
echo "Access your Station Graphs Web Dashboard at:"
echo "  Live Graphs: http://${LAN_IP}:${DEFAULT_PORT}/"
echo "  Web Settings: http://${LAN_IP}:${DEFAULT_PORT}/settings.html"
echo ""
echo "Service status: sudo systemctl status ${SERVICE_NAME}"
echo "Service logs:   sudo journalctl -u ${SERVICE_NAME} -f"
echo "=================================================================="

if [[ "${INSTALL_RADAR}" == "true" ]]; then
  info "Launching companion installation for ADSBItalia Station Radar..."
  bash <(curl -sL https://raw.githubusercontent.com/ADSBItalia/station-radar/main/install.sh) || true
fi
