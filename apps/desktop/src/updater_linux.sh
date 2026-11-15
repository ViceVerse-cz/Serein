#!/bin/sh
set -eu
stage=$1
installed=$2
parent=$3
commit=$4
# The old image's mount disappears after shutdown. Never reuse its runtime paths,
# bundled libraries, or sharun/uruntime helper state when launching the update.
old_appdir=${APPDIR:-}
old_appdir=${old_appdir%/}

strip_appdir_var() {
  var_name=$1
  prefix=$2
  eval "var_is_set=\${${var_name}+1}"
  [ -n "$var_is_set" ] || return 0
  eval "var_value=\$$var_name"
  [ -n "$var_value" ] || return 0
  filtered=""
  old_ifs=${IFS-}
  old_ifs_set=${IFS+1}
  old_opts=$-
  IFS=:
  set -f
  for entry in $var_value; do
    [ -n "$entry" ] || continue
    case "$entry" in
      "$prefix"|"$prefix"/*) continue ;;
    esac
    filtered="${filtered:+$filtered:}$entry"
  done
  case "$old_opts" in
    *f*) ;;
    *) set +f ;;
  esac
  if [ -n "$old_ifs_set" ]; then
    IFS=$old_ifs
  else
    unset IFS
  fi
  if [ -n "$filtered" ]; then
    export "$var_name=$filtered"
  else
    unset "$var_name"
  fi
}

if [ -n "${HOSTPATH:-}" ]; then
  export PATH="$HOSTPATH"
elif [ -n "$old_appdir" ]; then
  strip_appdir_var PATH "$old_appdir"
fi

if [ -n "${HOST_HOME:-}" ]; then
  export HOME="$HOST_HOME"
elif [ -n "${REAL_HOME:-}" ]; then
  export HOME="$REAL_HOME"
fi

if [ -n "${HOST_XDG_CACHE_HOME:-}" ]; then
  export XDG_CACHE_HOME="$HOST_XDG_CACHE_HOME"
elif [ -n "${REAL_XDG_CACHE_HOME:-}" ]; then
  export XDG_CACHE_HOME="$REAL_XDG_CACHE_HOME"
elif [ "${XDG_CACHE_HOME:-}" != "${XDG_CACHE_HOME%/AppImage-Cache}" ]; then
  unset XDG_CACHE_HOME
fi

if [ -n "${HOST_XDG_CONFIG_HOME:-}" ]; then
  export XDG_CONFIG_HOME="$HOST_XDG_CONFIG_HOME"
elif [ -n "${REAL_XDG_CONFIG_HOME:-}" ]; then
  export XDG_CONFIG_HOME="$REAL_XDG_CONFIG_HOME"
fi

if [ -n "${HOST_XDG_DATA_HOME:-}" ]; then
  export XDG_DATA_HOME="$HOST_XDG_DATA_HOME"
elif [ -n "${REAL_XDG_DATA_HOME:-}" ]; then
  export XDG_DATA_HOME="$REAL_XDG_DATA_HOME"
fi

if [ -n "${HOST_XDG_STATE_HOME:-}" ]; then
  export XDG_STATE_HOME="$HOST_XDG_STATE_HOME"
elif [ -n "${REAL_XDG_STATE_HOME:-}" ]; then
  export XDG_STATE_HOME="$REAL_XDG_STATE_HOME"
fi

if [ -n "$old_appdir" ]; then
  for var_name in \
    XDG_DATA_DIRS \
    GI_TYPELIB_PATH \
    GSETTINGS_SCHEMA_DIR \
    GIO_MODULE_DIR \
    GTK_PATH \
    GTK_EXE_PREFIX \
    GTK_DATA_PREFIX \
    GTK_IM_MODULE_FILE \
    GDK_PIXBUF_MODULE_FILE \
    GDK_PIXBUF_MODULEDIR \
    GST_PLUGIN_PATH \
    GST_PLUGIN_SYSTEM_PATH \
    GST_PLUGIN_SYSTEM_PATH_1_0 \
    GST_PLUGIN_SCANNER \
    WEBKIT_EXEC_PATH \
    WEBKIT_INJECTED_BUNDLE_PATH \
    PIPEWIRE_CONFIG_DIR \
    PIPEWIRE_MODULE_DIR \
    SPA_PLUGIN_DIR \
    ALSA_CONFIG_DIR \
    ALSA_PLUGIN_DIR \
    SSL_CERT_FILE \
    SSL_CERT_DIR \
    OPENSSL_CONF \
    GCONV_PATH \
    LOCPATH \
    XKB_CONFIG_ROOT \
    XLOCALEDIR
  do
    strip_appdir_var "$var_name" "$old_appdir"
  done
fi

unset \
  LD_LIBRARY_PATH LD_PRELOAD \
  APPIMAGE APPDIR ARGV0 ARG0 OWD \
  SHARUN_DIR SHARUN_BIN_NAME SHARUN_REEXEC_DEPTH SHARUN_WORKING_DIR \
  SHARUN_EXTRA_LIBRARY_PATH SHARUN_ALLOW_LD_PRELOAD SHARUN_ALLOW_SYS_VKICD \
  SHARUN_ALLOW_QT_PLUGIN_PATH SHARUN_OLD_KERNEL_COMPAT SHARUN_PRINTENV \
  URUNTIME URUNTIME_DIR URUNTIME_TARGET_DEV URUNTIME_TARGET_INO \
  URUNTIME_MOUNT URUNTIME_EXTRACT URUNTIME_CLEANUP \
  APPOFFSET APPIMAGE_ARCH APPIMAGE_UID \
  ANYLINUX_DO_NOT_LOAD_LIBS ANYLINUX_LIB_DEBUG \
  CROSS_LIBC_DLOPEN_ROOT CROSS_LIBC_DLOPEN_LIBDIR CROSS_LIBC_DLOPEN_DEBUG \
  HOSTPATH HOST_HOME HOST_KERNEL_VERSION \
  HOST_XDG_CACHE_HOME HOST_XDG_CONFIG_HOME HOST_XDG_DATA_HOME HOST_XDG_STATE_HOME \
  REAL_HOME REAL_XDG_CACHE_HOME REAL_XDG_CONFIG_HOME REAL_XDG_DATA_HOME REAL_XDG_STATE_HOME
printf '%s\n' "$$" > "$stage/helper-ready"
while [ ! -f "$commit" ]; do
  if ! /bin/kill -0 "$parent" 2>/dev/null; then
    [ -f "$commit" ] && break
    exit 0
  fi
  /bin/sleep 1
done
count=0
while /bin/kill -0 "$parent" 2>/dev/null; do
  count=$((count + 1)); [ "$count" -lt 120 ] || exit 0
  /bin/sleep 1
done
backup="$stage/previous.AppImage"
# Keep the old inode recoverable while replacing the public path atomically.
/bin/ln "$installed" "$backup" || exit 1
if ! /bin/mv -f "$stage/package.AppImage" "$installed"; then
  "$installed" &
  exit 1
fi
"$installed" &
launched=$!
/bin/sleep 2
if ! /bin/kill -0 "$launched" 2>/dev/null; then
  /bin/mv -f "$backup" "$installed"
  "$installed" &
  exit 1
fi
/bin/rm -rf "$stage"
