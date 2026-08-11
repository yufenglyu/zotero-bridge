<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  BookOpen,
  Database,
  FolderOpen,
  HelpCircle,
  Link,
  Monitor,
  Moon,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Save,
  Settings,
  Stethoscope,
  Sun,
  Terminal,
  Trash2,
} from "@lucide/vue";

interface LibraryView {
  kind: string;
  zotero_library_id: string;
  display_name: string;
  last_version: number;
  enabled: boolean;
  last_error: string | null;
}

interface StatusView {
  instance: string;
  item_count: number;
  library_count: number;
  pending_jobs: number;
  failed_jobs: number;
  last_sync_at: string | null;
  paused: boolean;
  zotero_running: boolean;
  libraries: LibraryView[];
  mirror_directories: MirrorDirectoryView[];
}

interface MirrorDirectoryView {
  platform: string;
  enabled: boolean;
  directory: string;
  extension: string;
  expected_files: number;
  actual_files: number;
  missing_files: number;
  orphan_files: number;
  stale_files: number;
  latest_created_at: string | null;
  latest_modified_at: string | null;
}

interface Config {
  app: { poll_interval_seconds: number; start_at_login: boolean; log_level: string };
  zotero: {
    api_base: string;
    request_timeout_seconds: number;
    include_user_library: boolean;
    group_mode: string;
  };
  search: {
    default_limit: number;
    maximum_limit: number;
    index_abstract: boolean;
    index_extra: boolean;
    store_raw_json: boolean;
    short_query_fallback: boolean;
  };
  mirror: {
    windows: { enabled: boolean; directory: string; template: string; uri_template: string };
    macos: { enabled: boolean; directory: string; template: string; uri_template: string };
  };
  maintenance: { optimize_after_updates: number; retain_logs_days: number };
  storage: { database: string };
}

interface PathsView {
  config_dir: string;
}

interface PrefsScan {
  profile: string;
  prefs_file: string;
  total: number;
  exportable: number;
  portable: number;
  paths: number;
  plugins: number;
  sensitive: number;
  groups: PrefGroup[];
  prefs: PrefEntry[];
}

interface PrefGroup {
  id: string;
  label: string;
  scope: string;
  plugin: string | null;
  total: number;
  exportable: number;
  paths: number;
  sensitive: number;
}

interface PrefEntry {
  key: string;
  value: string | number | boolean;
  kind: string;
  scope: string;
  plugin: string | null;
}

interface DraftPrefEntry extends PrefEntry {
  value_text: string;
  deleted: boolean;
  original_value: string | number | boolean;
}

interface RestorePreview {
  backup_path: string;
  backup_items: number;
  will_add: number;
  will_modify: number;
  unchanged: number;
  skipped_paths: number;
  path_items: PrefEntry[];
}

interface RestoreReport {
  applied: number;
  added: number;
  modified: number;
  unchanged: number;
  deleted: number;
  skipped_paths: number;
  backup_file: string;
}

type ThemeMode = "light" | "dark" | "system";
type PlatformKey = "windows" | "macos";

const THEME_KEY = "zotero-bridge-theme-mode";
const storedTheme = localStorage.getItem(THEME_KEY);
const themeMode = ref<ThemeMode>((storedTheme as ThemeMode) || "system");
const systemDark = ref(window.matchMedia("(prefers-color-scheme: dark)").matches);

function applyTheme() {
  const dark = themeMode.value === "dark" || (themeMode.value === "system" && systemDark.value);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

watch(themeMode, (m) => {
  localStorage.setItem(THEME_KEY, m);
  applyTheme();
});
watch(systemDark, applyTheme);

const tab = ref<"status" | "backup" | "settings">("status");
const status = ref<StatusView | null>(null);
const config = ref<Config | null>(null);
const paths = ref<PathsView | null>(null);
const savedSnapshot = ref("");
const doctorLines = ref<string[]>([]);
const doctorOpen = ref(false);
const prefsScan = ref<PrefsScan | null>(null);
const prefsSourcePath = ref("");
const prefsBackupPath = ref("");
const prefsRestorePath = ref("");
const prefsDraft = ref<DraftPrefEntry[]>([]);
const prefsDraftSource = ref<"current" | "backup">("current");
const groupKindFilters = ref<Record<string, string>>({});
const groupSearchFilters = ref<Record<string, string>>({});
const expandedPrefGroups = ref<string[]>([]);
const restorePaths = ref(false);
const pathMigrationOpen = ref(false);
const pathOverrides = ref<Record<string, string>>({});
const restorePreview = ref<RestorePreview | null>(null);
const restoreReport = ref<RestoreReport | null>(null);
const busy = ref(false);
const message = ref("");
const tauriAvailable = ref(true);

const dirty = computed(
  () => config.value !== null && JSON.stringify(config.value) !== savedSnapshot.value
);

const LEGACY_DEFAULT_TEMPLATE = "{primary_creator} - {year} - {title} -- {item_key}";
const followZotero = ref(true);
const customBackup = ref("");
const zoteroTpl = ref<string | null>(null);

const currentPlatformKey = computed<PlatformKey>(() => {
  const platform = navigator.platform.toLowerCase();
  return platform.includes("mac") ? "macos" : "windows";
});

const currentPlatformName = computed(() =>
  currentPlatformKey.value === "macos" ? "macOS" : "Windows"
);

const currentLinkExtension = computed(() =>
  currentPlatformKey.value === "macos" ? ".webloc" : ".url"
);

const activeMirrorConfig = computed(() => {
  if (!config.value) return null;
  return config.value.mirror[currentPlatformKey.value];
});

const currentMirrorDirectory = computed(() => {
  const dirs = status.value?.mirror_directories ?? [];
  return dirs.find((dir) => dir.platform === currentPlatformKey.value) ?? null;
});

const currentMirrorIssueCount = computed(() => {
  const dir = currentMirrorDirectory.value;
  if (!dir) return 0;
  return dir.missing_files + dir.orphan_files + dir.stale_files;
});

function prefMatchesGroupFilters(pref: DraftPrefEntry, groupId: string): boolean {
  const kind = groupKindFilters.value[groupId] || "all";
  const q = (groupSearchFilters.value[groupId] || "").trim().toLowerCase();
  if (pref.deleted) return false;
  if (kind !== "all" && pref.kind !== kind) return false;
  if (!q) return true;
  return (
    pref.key.toLowerCase().includes(q) ||
    pref.value_text.toLowerCase().includes(q) ||
    (pref.plugin || "").toLowerCase().includes(q)
  );
}

const activeDraftPrefs = computed(() =>
  prefsDraft.value.filter((pref) => !pref.deleted).map(toPrefEntry)
);

const groupedPrefs = computed(() => {
  const groups = new Map<
    string,
    {
      id: string;
      label: string;
      total: number;
      exportable: number;
      paths: number;
      sensitive: number;
      items: DraftPrefEntry[];
    }
  >();
  for (const pref of prefsDraft.value) {
    if (pref.deleted) continue;
    const id = groupIdForPref(pref);
    if (!groups.has(id)) {
      groups.set(id, {
        id,
        label: pref.plugin ? `插件：${pref.plugin}` : scopeLabel(pref.scope),
        total: 0,
        exportable: 0,
        paths: 0,
        sensitive: 0,
        items: [],
      });
    }
    const group = groups.get(id)!;
    group.total += 1;
    group.exportable += 1;
    if (pref.kind === "path") group.paths += 1;
    if (pref.kind === "sensitive") group.sensitive += 1;
    group.items.push(pref);
  }
  return Array.from(groups.values()).sort((a, b) => {
    const order = (id: string) => (id === "zotero" ? 0 : id === "browser" ? 1 : id === "unknown" ? 3 : 2);
    const byOrder = order(a.id) - order(b.id);
    return byOrder || a.label.localeCompare(b.label);
  });
});

const visiblePrefCount = computed(() =>
  groupedPrefs.value.reduce((sum, group) => sum + filteredGroupItems(group).length, 0)
);

const pathMigrationItems = computed(() =>
  prefsDraft.value.filter((pref) => !pref.deleted && pref.kind === "path")
);

function syncFollowState() {
  const tpl = activeMirrorConfig.value?.template ?? "";
  followZotero.value = tpl.trim() === "" || tpl.trim() === LEGACY_DEFAULT_TEMPLATE;
}

watch(followZotero, (follow) => {
  const mirror = activeMirrorConfig.value;
  if (!mirror) return;
  const cur = mirror.template;
  if (follow) {
    if (cur.trim() !== "" && cur.trim() !== LEGACY_DEFAULT_TEMPLATE) customBackup.value = cur;
    mirror.template = "";
  } else {
    mirror.template = customBackup.value || LEGACY_DEFAULT_TEMPLATE;
  }
});

let timer: number | undefined;
let themeTimer: number | undefined;
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
const onSystemTheme = (e: MediaQueryListEvent) => (systemDark.value = e.matches);

const connectionLabel = computed(() => {
  if (!status.value) return "读取中";
  if (status.value.paused) return "同步已暂停";
  if (status.value.zotero_running) return "Zotero 已连接";
  return "等待 Zotero";
});

const connectionTone = computed(() => {
  if (!status.value) return "neutral";
  if (status.value.paused) return "warn";
  if (status.value.zotero_running) return "ok";
  return "off";
});

const searchableNote = computed(() => {
  const count = status.value?.item_count ?? 0;
  return `${count.toLocaleString()} 条可搜索文献；顶层附件、笔记、批注按规格不进入索引。`;
});

const themeLabel = computed(() => {
  if (themeMode.value === "light") return "浅色";
  if (themeMode.value === "dark") return "深色";
  return "系统";
});

const footerMessage = computed(() => {
  if (!tauriAvailable.value) return "浏览器预览模式：桌面数据不可用";
  if (message.value) return message.value;
  if (busy.value) return "正在执行操作";
  if (doctorLines.value.length > 0) return doctorLines.value[doctorLines.value.length - 1];
  return status.value?.last_sync_at ? `最近同步 ${fmtTime(status.value.last_sync_at)}` : "就绪";
});

function cycleTheme() {
  themeMode.value =
    themeMode.value === "system" ? "light" : themeMode.value === "light" ? "dark" : "system";
}

async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    if (!("__TAURI_INTERNALS__" in window)) tauriAvailable.value = false;
    message.value = String(e);
    console.warn("invoke failed:", cmd, e);
    return null;
  }
}

async function refreshStatus() {
  const s = await safeInvoke<StatusView>("get_status");
  if (s) status.value = s;
}

async function syncNow() {
  busy.value = true;
  message.value = "";
  try {
    const r = await safeInvoke<string>("sync_now");
    if (r !== null) message.value = r;
  } finally {
    busy.value = false;
    await refreshStatus();
  }
}

async function togglePause() {
  if (!status.value) return;
  await safeInvoke("set_paused", { paused: !status.value.paused });
  await refreshStatus();
}

async function rebuildIndex() {
  busy.value = true;
  try {
    const r = await safeInvoke<string>("rebuild_index");
    if (r !== null) message.value = r;
  } finally {
    busy.value = false;
  }
}

async function refreshLinks() {
  busy.value = true;
  message.value = "";
  try {
    const r = await safeInvoke<string>("refresh_links");
    if (r !== null) message.value = r;
  } finally {
    busy.value = false;
    await refreshStatus();
  }
}

async function openDir(which: string) {
  await safeInvoke("open_dir", { which });
}

async function loadConfig() {
  const c = await safeInvoke<Config>("get_config");
  if (c) {
    config.value = c;
    savedSnapshot.value = JSON.stringify(c);
    syncFollowState();
  }
  zoteroTpl.value = await safeInvoke<string | null>("zotero_template");
  const p = await safeInvoke<PathsView>("get_paths");
  if (p) paths.value = p;
}

async function saveConfig() {
  if (!config.value) return;
  busy.value = true;
  try {
    const r = await safeInvoke<string>("save_config", { config: config.value });
    if (r !== null) {
      message.value = r;
      savedSnapshot.value = JSON.stringify(config.value);
    }
  } finally {
    busy.value = false;
  }
}

function autoSaveConfig() {
  if (activeMirrorConfig.value) activeMirrorConfig.value.enabled = true;
  if (!dirty.value || busy.value) return;
  void saveConfig();
}

async function runDoctor() {
  doctorOpen.value = true;
  busy.value = true;
  try {
    const lines = await safeInvoke<string[]>("doctor");
    if (lines) doctorLines.value = lines;
  } finally {
    busy.value = false;
  }
}

function restoreOptions() {
  applyPathOverridesToDraft();
  return { restore_paths: restorePaths.value || hasPathOverrides(), mappings: [] };
}

function hasPathOverrides(): boolean {
  return pathMigrationItems.value.some((pref) => {
    const next = (pathOverrides.value[pref.key] ?? "").trim();
    return next !== "" && next !== pref.value_text;
  });
}

function resetPathOverrides() {
  pathOverrides.value = {};
  for (const pref of pathMigrationItems.value) {
    pathOverrides.value[pref.key] = pref.value_text;
  }
}

function applyPathOverridesToDraft() {
  for (const pref of pathMigrationItems.value) {
    const next = pathOverrides.value[pref.key];
    if (next !== undefined && next.trim() !== "") {
      pref.value_text = next;
    }
  }
}

async function scanPrefs() {
  busy.value = true;
  try {
    const scan = await safeInvoke<PrefsScan>("scan_zotero_prefs", {
      path: prefsSourcePath.value.trim() || null,
    });
    if (scan) {
      prefsScan.value = scan;
      prefsSourcePath.value = scan.prefs_file;
      prefsDraft.value = scan.prefs.map(toDraftPref);
      prefsDraftSource.value = "current";
      restorePaths.value = false;
      resetPathOverrides();
      pruneExpandedPrefGroups();
      message.value = `已扫描 ${scan.total} 个 Zotero 设置项，可备份 ${scan.exportable} 项。`;
    }
  } finally {
    busy.value = false;
  }
}

async function backupPrefs() {
  busy.value = true;
  try {
    const path = await safeInvoke<string>("backup_zotero_prefs", {
      path: prefsBackupPath.value.trim() || null,
      sourcePath: prefsSourcePath.value.trim() || null,
      prefs: activeDraftPrefs.value,
    });
    if (path) {
      prefsBackupPath.value = path;
      prefsRestorePath.value = path;
      message.value = `Zotero 设置已备份：${path}`;
      await scanPrefs();
    }
  } finally {
    busy.value = false;
  }
}

async function loadPrefsBackup() {
  if (!prefsRestorePath.value.trim()) {
    message.value = "请先填写要导入的备份文件路径。";
    return;
  }
  busy.value = true;
  try {
    const backup = await safeInvoke<{ prefs: PrefEntry[] }>("load_zotero_prefs_backup", {
      path: prefsRestorePath.value.trim(),
    });
    if (backup) {
      prefsDraft.value = backup.prefs.map(toDraftPref);
      prefsDraftSource.value = "backup";
      restorePaths.value = false;
      pathMigrationOpen.value = true;
      resetPathOverrides();
      pruneExpandedPrefGroups();
      restorePreview.value = null;
      restoreReport.value = null;
      message.value = `已加载 ${backup.prefs.length} 个备份设置项，可查看、编辑或删除后再恢复。`;
    }
  } finally {
    busy.value = false;
  }
}

async function previewPrefsRestore() {
  if (!prefsRestorePath.value.trim()) {
    message.value = "请先填写要导入的备份文件路径。";
    return;
  }
  busy.value = true;
  try {
    const preview = await safeInvoke<RestorePreview>("preview_restore_zotero_prefs", {
      path: prefsRestorePath.value.trim(),
      options: restoreOptions(),
      prefs: activeDraftPrefs.value,
      targetPath: prefsSourcePath.value.trim() || null,
    });
    if (preview) {
      restorePreview.value = preview;
      restoreReport.value = null;
      message.value = `恢复预览：新增 ${preview.will_add}，修改 ${preview.will_modify}，路径跳过 ${preview.skipped_paths}。`;
    }
  } finally {
    busy.value = false;
  }
}

async function applyPrefsRestore() {
  if (!restorePreview.value || !prefsRestorePath.value.trim()) {
    message.value = "请先生成恢复预览。";
    return;
  }
  busy.value = true;
  try {
    const report = await safeInvoke<RestoreReport>("restore_zotero_prefs", {
      path: prefsRestorePath.value.trim(),
      options: restoreOptions(),
      prefs: activeDraftPrefs.value,
      targetPath: prefsSourcePath.value.trim() || null,
    });
    if (report) {
      restoreReport.value = report;
      message.value = `恢复完成：新增 ${report.added}，修改 ${report.modified}，当前 prefs 已备份。`;
      await scanPrefs();
    }
  } finally {
    busy.value = false;
  }
}

async function savePrefsDraft() {
  if (prefsDraftSource.value === "backup") {
    message.value = "导入备份请先预览，再应用恢复；写回只用于本机扫描草稿。";
    return;
  }
  if (!prefsDraft.value.length) {
    message.value = "请先扫描或加载设置项。";
    return;
  }
  busy.value = true;
  try {
    const report = await safeInvoke<RestoreReport>("save_zotero_prefs", {
      path: prefsSourcePath.value.trim() || null,
      prefs: activeDraftPrefs.value,
    });
    if (report) {
      restoreReport.value = report;
      message.value = `写回完成：新增 ${report.added}，修改 ${report.modified}，删除 ${report.deleted}，当前 prefs 已备份。`;
      await scanPrefs();
    }
  } finally {
    busy.value = false;
  }
}

async function browseBackupPath() {
  const selected = await safeInvoke<string | null>("browse_backup_file", {
    defaultDir: prefsSourcePath.value.trim() || null,
  });
  if (selected) prefsBackupPath.value = selected;
}

async function browseRestorePath() {
  const selected = await safeInvoke<string | null>("browse_restore_file", {
    defaultDir: prefsBackupPath.value.trim() || prefsSourcePath.value.trim() || null,
  });
  if (selected) {
    prefsRestorePath.value = selected;
    await loadPrefsBackup();
  }
}

function toDraftPref(pref: PrefEntry): DraftPrefEntry {
  return {
    ...pref,
    value_text: String(pref.value),
    original_value: pref.value,
    deleted: false,
  };
}

function toPrefEntry(pref: DraftPrefEntry): PrefEntry {
  return {
    key: pref.key,
    value: coercePrefValue(pref.value_text, pref.original_value),
    kind: pref.kind,
    scope: pref.scope,
    plugin: pref.plugin,
  };
}

function coercePrefValue(value: string, original: string | number | boolean): string | number | boolean {
  if (typeof original === "boolean") return value.trim().toLowerCase() === "true";
  if (typeof original === "number") {
    const n = Number(value);
    return Number.isFinite(n) ? n : original;
  }
  return value;
}

function groupIdForPref(pref: Pick<PrefEntry, "scope" | "plugin">): string {
  if (pref.scope === "plugin") return `plugin:${pref.plugin || "unknown"}`;
  if (pref.scope === "zotero") return "zotero";
  if (pref.scope === "browser") return "browser";
  return "unknown";
}

function scopeLabel(scope: string): string {
  if (scope === "zotero") return "Zotero 本身";
  if (scope === "browser") return "Zotero 框架";
  return "未知来源";
}

function removeDraftPref(pref: DraftPrefEntry) {
  pref.deleted = true;
}

function filteredGroupItems(group: { id: string; items: DraftPrefEntry[] }): DraftPrefEntry[] {
  return group.items.filter((pref) => prefMatchesGroupFilters(pref, group.id));
}

function pruneExpandedPrefGroups() {
  const valid = new Set(prefsDraft.value.filter((pref) => !pref.deleted).map(groupIdForPref));
  expandedPrefGroups.value = expandedPrefGroups.value.filter((id) => valid.has(id));
  expandedPrefGroups.value.forEach(ensureGroupFilter);
}

function isGroupExpanded(id: string): boolean {
  return expandedPrefGroups.value.includes(id);
}

function ensureGroupFilter(id: string) {
  if (!groupKindFilters.value[id]) groupKindFilters.value[id] = "all";
  if (groupSearchFilters.value[id] === undefined) groupSearchFilters.value[id] = "";
}

function togglePrefGroup(id: string) {
  if (isGroupExpanded(id)) {
    expandedPrefGroups.value = expandedPrefGroups.value.filter((item) => item !== id);
  } else {
    ensureGroupFilter(id);
    expandedPrefGroups.value = [...expandedPrefGroups.value, id];
  }
}

function fmtTime(iso: string | null): string {
  if (!iso) return "从未";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

onMounted(async () => {
  applyTheme();
  darkQuery.addEventListener("change", onSystemTheme);
  themeTimer = window.setInterval(() => {
    if (themeMode.value === "system") systemDark.value = darkQuery.matches;
  }, 1000);

  await Promise.all([refreshStatus(), loadConfig()]);
  await scanPrefs();
  timer = window.setInterval(refreshStatus, 3000);
});

onUnmounted(() => {
  if (timer) window.clearInterval(timer);
  if (themeTimer) window.clearInterval(themeTimer);
  darkQuery.removeEventListener("change", onSystemTheme);
});
</script>

<template>
  <div class="shell">
    <aside class="sidebar">
      <div class="brand">
        <div>
          <h1>Zotero Bridge</h1>
          <p>Zotero 本地增强工具</p>
        </div>
      </div>

      <nav class="nav">
        <button :class="{ active: tab === 'status' }" @click="tab = 'status'">
          <span><Activity class="icon" />索引链接</span>
          <em v-if="status">{{ status.item_count.toLocaleString() }}</em>
        </button>
        <button :class="{ active: tab === 'backup' }" @click="tab = 'backup'">
          <span><Database class="icon" />设置备份</span>
          <em v-if="prefsScan">{{ prefsScan.exportable.toLocaleString() }}</em>
        </button>
      </nav>

      <div class="sidebar-bottom">
        <button class="side-tool" :title="`当前主题：${themeLabel}`" @click="cycleTheme">
          <span>
            <Sun v-if="themeMode === 'light'" class="icon" />
            <Moon v-else-if="themeMode === 'dark'" class="icon" />
            <Monitor v-else class="icon" />
            {{ themeLabel }}
          </span>
        </button>
        <button class="side-tool" :class="{ active: tab === 'settings' }" @click="tab = 'settings'">
          <span><Settings class="icon" />设置</span>
          <i v-if="dirty" title="有未保存的修改"></i>
        </button>
      </div>
    </aside>

    <main class="main">
      <section v-if="tab === 'status' && status" class="page">
        <div class="hero">
          <div>
            <span class="eyebrow">Library Console</span>
            <h2>文献索引与链接定位</h2>
            <p>{{ searchableNote }}</p>
          </div>
          <div class="hero-actions">
            <button :disabled="busy" @click="runDoctor">
              <Stethoscope class="icon" />诊断
            </button>
            <span class="hint-icon" title="检查 Zotero Local API、SQLite FTS5、链接目录和本地数据库。">
              <HelpCircle class="icon" />
            </span>
          </div>
        </div>

        <div class="content-grid">
          <section class="panel span-12">
            <div class="panel-head">
              <div>
                <h3>运行概览</h3>
                <p>集中查看 Zotero 实例、文献库、链接文件和本地任务状态。</p>
              </div>
            </div>
            <div class="overview-grid">
              <div class="overview-card">
                <span>已索引条目</span>
                <strong>{{ status.item_count.toLocaleString() }}</strong>
                <small>最近同步 {{ fmtTime(status.last_sync_at) }}</small>
              </div>
              <div class="overview-card">
                <span>链接文件</span>
                <strong v-if="currentMirrorDirectory">
                  {{ currentMirrorDirectory.actual_files.toLocaleString() }} / {{ currentMirrorDirectory.expected_files.toLocaleString() }}
                </strong>
                <strong v-else>未配置</strong>
                <small v-if="currentMirrorDirectory">
                  {{ currentMirrorIssueCount === 0 ? "正常" : `${currentMirrorIssueCount} 项异常` }} · 更新 {{ fmtTime(currentMirrorDirectory.latest_modified_at) }}
                </small>
                <small v-else>未生成链接目录</small>
              </div>
              <div class="overview-card">
                <span>Zotero 实例</span>
                <strong>{{ status.instance || "未知" }}</strong>
                <small>{{ status.library_count }} 个文献库 · 排除附件、笔记、批注</small>
              </div>
            </div>
            <section class="overview-block">
              <h4>文献库状态</h4>
              <div class="library-table compact-library-table">
                <div
                  v-for="lib in status.libraries"
                  :key="lib.kind + lib.zotero_library_id"
                  class="library-row compact-library-row"
                >
                  <span class="tag">{{ lib.kind }}</span>
                  <strong>{{ lib.display_name }}</strong>
                  <em>id={{ lib.zotero_library_id }}</em>
                  <em>版本 {{ lib.last_version }}</em>
                  <span class="state" :class="{ off: !lib.enabled }">
                    {{ lib.enabled ? "启用" : "停用" }}
                  </span>
                  <p v-if="lib.last_error">{{ lib.last_error }}</p>
                </div>
              </div>
            </section>
            <div class="queue-strip">
              <span>任务队列</span>
              <strong>待写入 {{ status.pending_jobs.toLocaleString() }}</strong>
              <strong :class="{ warn: status.failed_jobs > 0 }">失败 {{ status.failed_jobs.toLocaleString() }}</strong>
            </div>
            <div class="toolbar">
              <button
                class="primary"
                :disabled="busy"
                title="立即向 Zotero Local API 拉取变化，更新本地索引，并把本轮变化对应的链接文件写入、改名或删除。"
                @click="syncNow"
              >
                <RefreshCw class="icon" />立即同步
              </button>
              <button :disabled="busy" @click="togglePause">
                <Play v-if="status.paused" class="icon" />
                <Pause v-else class="icon" />
                {{ status.paused ? "恢复同步" : "暂停同步" }}
              </button>
              <button
                :disabled="busy"
                title="不重新拉取 Zotero 数据，只按当前索引、命名模板和 URI 模板重算链接文件；会补写缺失文件、覆盖内容变化的文件、改名已重命名文献对应的文件。"
                @click="refreshLinks"
              >
                <Link class="icon" />刷新链接
              </button>
              <button class="danger" :disabled="busy" @click="rebuildIndex">
                <RotateCcw class="icon" />重建索引
              </button>
            </div>
          </section>

          <section v-if="activeMirrorConfig" class="panel span-12">
            <div class="panel-head">
              <div>
                <h3><Link class="icon" />链接文件</h3>
                <p>配置当前系统的链接文件目录、命名模板和 URI 模板；文件数量与任务状态在运行概览中查看。</p>
              </div>
            </div>
            <div v-if="currentMirrorDirectory" class="mirror-table">
              <div class="mirror-row mirror-directory-row">
                <div class="mirror-directory">
                  <strong>目录设置</strong>
                  <input
                    type="text"
                    v-model="activeMirrorConfig.directory"
                    spellcheck="false"
                    @change="autoSaveConfig"
                    @blur="autoSaveConfig"
                    @keyup.enter="autoSaveConfig"
                  />
                  <button type="button" class="compact-icon" title="打开链接文件目录" @click="openDir('mirror')">
                    <FolderOpen class="icon" />
                  </button>
                </div>
              </div>
            </div>
            <div class="link-settings-stack">
              <section class="settings-section compact-section">
                <h4>命名模板</h4>
                <label class="check"><input type="checkbox" v-model="followZotero" @change="autoSaveConfig" />跟随 Zotero 命名模板</label>
                <label v-if="!followZotero" class="field field-top">
                  <span>自定义模板</span>
                  <textarea
                    rows="4"
                    spellcheck="false"
                    v-model="activeMirrorConfig.template"
                    @change="autoSaveConfig"
                    @blur="autoSaveConfig"
                    @keyup.ctrl.enter="autoSaveConfig"
                  ></textarea>
                </label>
              </section>
              <section class="settings-section compact-section">
                <h4>URI 模板</h4>
                <label class="field">
                  <span class="label-with-help">{{ currentPlatformName }} URI<span class="hint-icon" title="写入链接文件的 URI。默认 {select_uri}，可用占位符：{select_uri}、{item_key}、{itemKey}、{title}。"><HelpCircle class="icon" /></span></span>
                  <input
                    type="text"
                    v-model="activeMirrorConfig.uri_template"
                    placeholder="{select_uri}"
                    @change="autoSaveConfig"
                    @blur="autoSaveConfig"
                    @keyup.enter="autoSaveConfig"
                  />
                </label>
              </section>
            </div>
          </section>

        </div>
      </section>

      <section v-if="tab === 'backup'" class="page">
        <div class="hero">
          <div>
            <span class="eyebrow">Preferences Backup</span>
            <h2>设置备份与迁移</h2>
            <p>备份 Zotero 本身和插件写入 prefs.js 的设置项；恢复前可预览变化并映射路径。</p>
          </div>
        </div>

        <section class="panel settings-card">
          <h3><Database class="icon" />设置概览</h3>
          <div class="backup-overview">
            <div class="backup-summary">
              <div>
                <span>设置总数</span>
                <strong>{{ prefsScan ? prefsScan.total.toLocaleString() : "..." }}</strong>
              </div>
              <div>
                <span>可备份</span>
                <strong>{{ prefsScan ? prefsScan.exportable.toLocaleString() : "..." }}</strong>
              </div>
              <div>
                <span>路径项</span>
                <strong>{{ prefsScan ? prefsScan.paths.toLocaleString() : "..." }}</strong>
              </div>
              <div>
                <span>敏感项</span>
                <strong>{{ prefsScan ? prefsScan.sensitive.toLocaleString() : "..." }}</strong>
              </div>
            </div>

            <div class="backup-actions">
              <label class="field wide">
                <span>源配置</span>
                <input type="text" v-model="prefsSourcePath" placeholder="填写 prefs.js 文件或 Zotero profile 目录" />
                <button type="button" :disabled="busy" @click="scanPrefs">
                  <RefreshCw class="icon" />扫描
                </button>
                <button type="button" :disabled="busy || prefsDraftSource === 'backup'" @click="savePrefsDraft">
                  <Save class="icon" />写回
                </button>
              </label>
              <label class="field wide">
                <span>备份文件</span>
                <input type="text" v-model="prefsBackupPath" placeholder="留空则保存到 Zotero profile" />
                <button type="button" :disabled="busy" @click="browseBackupPath">
                  <FolderOpen class="icon" />浏览
                </button>
                <button type="button" :disabled="busy" @click="backupPrefs">
                  <Save class="icon" />备份
                </button>
              </label>
              <label class="field wide">
                <span>导入文件</span>
                <input type="text" v-model="prefsRestorePath" placeholder="填写 zotero-bridge-prefs-backup-*.json" />
                <button type="button" :disabled="busy" @click="browseRestorePath">
                  <FolderOpen class="icon" />浏览
                </button>
                <button type="button" :disabled="busy" @click="previewPrefsRestore">
                  <RefreshCw class="icon" />预览
                </button>
              </label>
            </div>
          </div>
        </section>

        <section class="panel settings-card">
          <div class="panel-head">
            <div>
              <h3><BookOpen class="icon" />设置分组</h3>
              <p>{{ prefsDraftSource === "backup" ? "当前显示导入备份中的设置项。" : "当前显示本机 Zotero 可备份设置项。" }}编辑和删除会先进入草稿，点击写回后更新源 prefs.js。</p>
            </div>
            <div class="pref-count">{{ visiblePrefCount }} / {{ activeDraftPrefs.length }}</div>
          </div>
          <div class="pref-groups">
            <div v-for="group in groupedPrefs" :key="group.id" class="pref-group">
              <button class="pref-group-row" @click="togglePrefGroup(group.id)">
                <strong>{{ isGroupExpanded(group.id) ? "−" : "+" }} {{ group.label }}</strong>
                <span>{{ group.exportable }} 可备份 / {{ group.total }} 总计</span>
                <em>路径 {{ group.paths }}</em>
                <em>敏感 {{ group.sensitive }}</em>
              </button>
              <div v-if="isGroupExpanded(group.id)" class="pref-table">
                <div class="pref-filters inline-pref-filters">
                  <select v-model="groupKindFilters[group.id]">
                    <option value="all">全部类型</option>
                    <option value="portable">普通项</option>
                    <option value="path">路径项</option>
                    <option value="unknown">未知项</option>
                  </select>
                  <input type="text" v-model="groupSearchFilters[group.id]" placeholder="在当前分组中搜索 key 或 value" />
                </div>
                <div class="pref-row pref-head">
                  <span>类型</span>
                  <span>设置项</span>
                  <span>值</span>
                  <span></span>
                </div>
                <div v-for="pref in filteredGroupItems(group).slice(0, 200)" :key="pref.key" class="pref-row">
                  <span>{{ pref.kind }}</span>
                  <strong :title="pref.key">{{ pref.key }}</strong>
                  <input v-model="pref.value_text" spellcheck="false" />
                  <button class="danger compact-icon" title="从本次草稿删除" @click="removeDraftPref(pref)">
                    <Trash2 class="icon" />
                  </button>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section class="panel settings-card">
          <div class="panel-head tight-head">
            <div>
              <h3><FolderOpen class="icon" />路径迁移</h3>
              <p>导入备份时逐项核对路径设置；填写新路径后，预览和恢复会用新路径覆盖备份中的旧路径。</p>
            </div>
            <div class="toolbar compact">
              <label class="check inline-check"><input type="checkbox" v-model="restorePaths" />导入未修改的路径项</label>
              <button type="button" :disabled="!pathMigrationItems.length" @click="pathMigrationOpen = !pathMigrationOpen">
                {{ pathMigrationOpen ? "收起列表" : "展开列表" }}
              </button>
            </div>
          </div>
          <div v-if="pathMigrationOpen && pathMigrationItems.length" class="path-migration-list">
            <div class="path-migration-row path-migration-head">
              <span>设置项</span>
              <span>旧路径</span>
              <span>新路径</span>
            </div>
            <div v-for="item in pathMigrationItems" :key="item.key" class="path-migration-row">
              <strong :title="item.key">{{ item.plugin || item.scope }} · {{ item.key }}</strong>
              <code :title="item.value_text">{{ item.value_text }}</code>
              <input v-model="pathOverrides[item.key]" spellcheck="false" placeholder="填写当前电脑上的路径" />
            </div>
          </div>
          <div v-else class="template-box compact-template">
            当前草稿中没有路径项。
          </div>
        </section>

        <section v-if="restorePreview" class="panel settings-card">
          <h3><RotateCcw class="icon" />恢复预览</h3>
          <div class="preview-strip">
            <span>新增 <strong>{{ restorePreview.will_add }}</strong></span>
            <span>修改 <strong>{{ restorePreview.will_modify }}</strong></span>
            <span>不变 <strong>{{ restorePreview.unchanged }}</strong></span>
            <span>路径跳过 <strong>{{ restorePreview.skipped_paths }}</strong></span>
          </div>
          <div v-if="restorePreview.path_items.length" class="path-list">
            <div v-for="item in restorePreview.path_items.slice(0, 6)" :key="item.key">
              <strong>{{ item.plugin || item.scope }}</strong>
              <span>{{ item.key }}</span>
            </div>
          </div>
          <div class="toolbar compact">
            <button class="primary" :disabled="busy" @click="applyPrefsRestore">
              <RotateCcw class="icon" />应用恢复
            </button>
          </div>
        </section>

        <div v-if="restoreReport" class="template-box compact-template">
          当前 prefs 备份：{{ restoreReport.backup_file }}
        </div>
      </section>

      <section v-if="tab === 'settings' && config" class="page">
        <div class="page-title compact-title">
          <h2>设置</h2>
        </div>

        <div class="settings-stack">
          <section class="panel settings-card">
            <div class="settings-form">
              <label class="check inline-check"><input type="checkbox" v-model="config.app.start_at_login" />开机自动启动</label>
              <label class="check inline-check"><input type="checkbox" v-model="config.zotero.include_user_library" />索引个人库</label>

              <label class="field">
                <span>Local API</span>
                <input type="text" v-model="config.zotero.api_base" />
              </label>
              <label class="field">
                <span class="label-with-help">轮询周期<span class="hint-icon" title="后台自动检查 Zotero 变化的间隔，单位为秒。建议 15-60 秒。"><HelpCircle class="icon" /></span></span>
                <input type="number" v-model.number="config.app.poll_interval_seconds" min="5" />
              </label>
              <label class="field">
                <span class="label-with-help">请求超时<span class="hint-icon" title="访问 Zotero Local API 的超时时间，单位为秒。"><HelpCircle class="icon" /></span></span>
                <input type="number" v-model.number="config.zotero.request_timeout_seconds" min="1" />
              </label>
              <label class="field">
                <span>群组库</span>
                <select v-model="config.zotero.group_mode"><option value="all">全部索引</option><option value="none">不索引</option></select>
              </label>
              <label class="field">
                <span>日志级别</span>
                <select v-model="config.app.log_level"><option value="error">error</option><option value="warn">warn</option><option value="info">info</option><option value="debug">debug</option></select>
              </label>
              <label class="field wide">
                <span>配置目录</span>
                <input type="text" :value="paths?.config_dir || ''" readonly />
                <button type="button" title="打开配置目录" @click="openDir('config')">
                  <FolderOpen class="icon" />打开
                </button>
              </label>
            </div>
            <div class="inline-save">
              <span :class="{ ok: !dirty }">{{ dirty ? "有未保存的修改" : "设置已保存" }}</span>
              <button class="primary" :disabled="busy || !dirty" @click="saveConfig">
                <Save class="icon" />保存设置
              </button>
            </div>
          </section>
        </div>
      </section>

    </main>

    <div v-if="doctorOpen" class="modal-backdrop" @click.self="doctorOpen = false">
      <section class="modal-panel">
        <div class="panel-head">
          <div>
            <h3><Stethoscope class="icon" />诊断结果</h3>
            <p>检查 Zotero Local API、SQLite FTS5、链接目录和本地数据库。</p>
          </div>
          <button type="button" @click="doctorOpen = false">关闭</button>
        </div>
        <pre v-if="doctorLines.length" class="doctor modal-doctor">{{ doctorLines.join("\n") }}</pre>
        <div v-else class="template-box compact-template">正在运行诊断...</div>
      </section>
    </div>

    <footer class="statusbar">
      <span class="statusbar-item" :class="connectionTone">
        <b></b>{{ connectionLabel }}
      </span>
      <span class="statusbar-item">
        <Database class="icon" />{{ status ? status.item_count.toLocaleString() : 0 }} 条索引
      </span>
      <span class="statusbar-message">{{ footerMessage }}</span>
      <button class="statusbar-button" @click="openDir('logs')">
        <Terminal class="icon" />日志
      </button>
    </footer>
  </div>
</template>
