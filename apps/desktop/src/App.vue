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

const tab = ref<"status" | "settings" | "doctor">("status");
const status = ref<StatusView | null>(null);
const config = ref<Config | null>(null);
const paths = ref<PathsView | null>(null);
const savedSnapshot = ref("");
const doctorLines = ref<string[]>([]);
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
    tauriAvailable.value = false;
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

async function runDoctor() {
  busy.value = true;
  try {
    const lines = await safeInvoke<string[]>("doctor");
    if (lines) doctorLines.value = lines;
  } finally {
    busy.value = false;
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
          <p>本地文献检索桥</p>
        </div>
      </div>

      <nav class="nav">
        <button :class="{ active: tab === 'status' }" @click="tab = 'status'">
          <span><Activity class="icon" />状态</span>
          <em v-if="status">{{ status.item_count.toLocaleString() }}</em>
        </button>
        <button :class="{ active: tab === 'doctor' }" @click="tab = 'doctor'">
          <span><Stethoscope class="icon" />诊断</span>
          <em v-if="status && status.failed_jobs > 0">{{ status.failed_jobs }}</em>
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
        </div>

        <div class="metric-grid">
          <article class="metric">
            <span>已索引条目</span>
            <strong>{{ status.item_count.toLocaleString() }}</strong>
            <small>排除附件、笔记、批注</small>
          </article>
          <article class="metric">
            <span>文献库</span>
            <strong>{{ status.library_count }}</strong>
            <small>个人库 + 群组库</small>
          </article>
          <article class="metric">
            <span>待写入链接</span>
            <strong>{{ status.pending_jobs }}</strong>
            <small>快捷目录任务队列</small>
          </article>
          <article class="metric" :class="{ danger: status.failed_jobs > 0 }">
            <span>失败任务</span>
            <strong>{{ status.failed_jobs }}</strong>
            <small>多次重试仍失败</small>
          </article>
        </div>

        <div class="content-grid">
          <section class="panel span-12">
            <div class="panel-head">
              <div>
                <h3>文献库</h3>
                <p>每个 Zotero 库独立记录版本和启用状态。</p>
              </div>
            </div>
            <div class="library-table">
              <div
                v-for="lib in status.libraries"
                :key="lib.kind + lib.zotero_library_id"
                class="library-row"
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

          <section class="panel span-12">
            <div class="panel-head">
              <div>
                <h3>链接文件目录</h3>
                <p>按当前索引、命名模板和 URI 模板检查本地链接文件是否一致。</p>
              </div>
            </div>
            <div v-if="currentMirrorDirectory" class="mirror-table">
              <button
                class="mirror-row"
                :class="{
                  disabled: !currentMirrorDirectory.enabled,
                  stale:
                    currentMirrorIssueCount > 0,
                }"
                title="打开链接文件目录"
                @click="openDir('mirror')"
              >
                <div class="mirror-main">
                  <strong>链接文件目录</strong>
                  <span>{{ currentMirrorDirectory.directory }}</span>
                </div>
                <div class="mirror-stats">
                  <strong>{{ currentMirrorDirectory.actual_files.toLocaleString() }} / {{ currentMirrorDirectory.expected_files.toLocaleString() }}</strong>
                  <span>本地文件 / 应有文件</span>
                </div>
                <div class="mirror-time">
                  <strong :class="{ warn: currentMirrorIssueCount > 0 }">
                    {{ currentMirrorIssueCount === 0 ? "正常" : `${currentMirrorIssueCount} 项异常` }}
                  </strong>
                  <span>最近更新 {{ fmtTime(currentMirrorDirectory.latest_modified_at) }}</span>
                </div>
                <FolderOpen class="icon" />
              </button>
            </div>
          </section>

          <section class="panel span-12">
            <div class="panel-head">
              <div>
                <h3>同步状态</h3>
                <p>绑定 Zotero 实例并持续刷新本地索引。</p>
              </div>
            </div>
            <dl class="facts">
              <div>
                <dt>当前实例</dt>
                <dd>{{ status.instance || "未知" }}</dd>
              </div>
              <div>
                <dt>最近同步</dt>
                <dd>{{ fmtTime(status.last_sync_at) }}</dd>
              </div>
            </dl>
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
        </div>
      </section>

      <section v-if="tab === 'settings' && config" class="page">
        <div class="page-title compact-title">
          <h2>设置</h2>
        </div>

        <div class="settings-stack">
          <section class="panel settings-card">
            <h3><Settings class="icon" />基础运行</h3>
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
              <label class="field">
                <span>配置目录</span>
                <input type="text" :value="paths?.config_dir || ''" readonly />
                <button type="button" title="打开配置目录" @click="openDir('config')">
                  <FolderOpen class="icon" />打开
                </button>
              </label>
            </div>
          </section>

          <section v-if="activeMirrorConfig" class="panel settings-card">
            <h3><Link class="icon" />链接文件</h3>
            <div class="settings-section">
              <h4>目录设置</h4>
              <div class="platform-box">
                <label class="check"><input type="checkbox" v-model="activeMirrorConfig.enabled" />启用 {{ currentLinkExtension }} 链接</label>
                <label class="field">
                  <span>链接文件目录</span>
                  <input type="text" v-model="activeMirrorConfig.directory" />
                  <button type="button" title="打开链接文件目录" @click="openDir('mirror')">
                    <FolderOpen class="icon" />打开
                  </button>
                </label>
              </div>
            </div>
            <div class="settings-section">
              <h4>命名模板</h4>
              <label class="check"><input type="checkbox" v-model="followZotero" />跟随 Zotero 命名模板</label>
              <label v-if="!followZotero" class="field field-top">
                <span>自定义模板</span>
                <textarea rows="5" spellcheck="false" v-model="activeMirrorConfig.template"></textarea>
              </label>
            </div>
            <div class="settings-section">
              <h4>URI 模板</h4>
              <label class="field">
                <span class="label-with-help">{{ currentPlatformName }} URI<span class="hint-icon" title="写入链接文件的 URI。默认 {select_uri}，可用占位符：{select_uri}、{item_key}、{itemKey}、{title}。"><HelpCircle class="icon" /></span></span>
                <input type="text" v-model="activeMirrorConfig.uri_template" placeholder="{select_uri}" />
              </label>
            </div>
          </section>
        </div>

        <div class="save-bar">
          <span :class="{ ok: !dirty }">{{ dirty ? "有未保存的修改" : "设置已保存" }}</span>
          <button class="primary" :disabled="busy || !dirty" @click="saveConfig">
            <Save class="icon" />保存设置
          </button>
        </div>
      </section>

      <section v-if="tab === 'doctor'" class="page">
        <div class="page-title">
          <span class="eyebrow">Doctor</span>
          <h2>环境诊断</h2>
          <p>检查 Zotero Local API、SQLite FTS5、快捷目录和本地数据库。</p>
        </div>
        <section class="panel">
          <div class="toolbar compact">
            <button class="primary" :disabled="busy" @click="runDoctor">
              <Stethoscope class="icon" />运行诊断
            </button>
          </div>
          <pre v-if="doctorLines.length" class="doctor">{{ doctorLines.join("\n") }}</pre>
        </section>
      </section>
    </main>

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
