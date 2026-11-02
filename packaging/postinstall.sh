#!/bin/sh
# Post-install / post-remove desktop & icon cache refresh for lumaprint (.deb / .rpm)
# SPDX-License-Identifier: GPL-3.0-or-later
set -e

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
