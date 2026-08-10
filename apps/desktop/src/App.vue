<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  BookOpen,
  Database,
  FileText,
  FolderOpen,
  HardDrive,
  HelpCircle,
  Link,
  Monitor,
  Moon,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
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

type ThemeMode = "light" | "dark" | "system";

const THEME_KEY = "zotero-bridge-theme-mode";
const LEGACY_THEME_KEY = "zsb-theme-mode";
const storedTheme = localStorage.getItem(THEME_KEY) || localStorage.getItem(LEGACY_THEME_KEY);
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

function syncFollowState() {
  const tpl = config.value?.mirror.windows.template ?? "";
  followZotero.value = tpl.trim() === "" || tpl.trim() === LEGACY_DEFAULT_TEMPLATE;
}

watch(followZotero, (follow) => {
  if (!config.value) return;
  const cur = config.value.mirror.windows.template;
  if (follow) {
    if (cur.trim() !== "" && cur.trim() !== LEGACY_DEFAULT_TEMPLATE) customBackup.value = cur;
    config.value.mirror.windows.template = "";
  } else {
    config.value.mirror.windows.template = customBackup.value || LEGACY_DEFAULT_TEMPLATE;
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
            <div class="mirror-table">
              <div
                v-for="dir in status.mirror_directories"
                :key="dir.platform"
                class="mirror-row"
                :class="{ disabled: !dir.enabled, stale: dir.missing_files + dir.orphan_files + dir.stale_files > 0 }"
              >
                <div class="mirror-main">
                  <strong>{{ dir.platform === "windows" ? "Windows" : "macOS" }}</strong>
                  <span>{{ dir.directory }}</span>
                </div>
                <div class="mirror-stats">
                  <span>文件 {{ dir.actual_files.toLocaleString() }} / {{ dir.expected_files.toLocaleString() }}</span>
                  <span>缺失 {{ dir.missing_files }}</span>
                  <span>孤儿 {{ dir.orphan_files }}</span>
                  <span>过期 {{ dir.stale_files }}</span>
                </div>
                <div class="mirror-time">
                  <span>创建 {{ fmtTime(dir.latest_created_at) }}</span>
                  <span>更新 {{ fmtTime(dir.latest_modified_at) }}</span>
                </div>
                <em>{{ dir.enabled ? "." + dir.extension : "未启用" }}</em>
              </div>
            </div>
          </section>

          <section class="panel span-7">
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

          <section class="panel span-5">
            <div class="panel-head">
              <div>
                <h3>目录</h3>
                <p>打开常用位置，便于配置 Listary。</p>
              </div>
            </div>
            <div class="action-list">
              <button @click="openDir('mirror')">
                <span><FolderOpen class="icon" />链接文件目录</span><em>.url / .webloc</em>
              </button>
              <button @click="openDir('config')">
                <span><FileText class="icon" />配置文件目录</span><em>config.toml</em>
              </button>
            </div>
          </section>
        </div>
      </section>

      <section v-if="tab === 'settings' && config" class="page">
        <div class="page-title compact-title">
          <h2>设置</h2>
        </div>

        <div class="settings-grid">
          <section class="panel">
            <h3><Settings class="icon" />常规</h3>
            <label class="check"><input type="checkbox" v-model="config.app.start_at_login" />开机自动启动</label>
            <label class="field">
              <span class="label-with-help">轮询周期<span class="hint-icon" title="后台自动检查 Zotero 变化的间隔，单位为秒。建议 15-60 秒。"><HelpCircle class="icon" /></span></span>
              <input type="number" v-model.number="config.app.poll_interval_seconds" min="5" />
            </label>
            <label class="field"><span>日志级别</span><select v-model="config.app.log_level"><option value="error">error</option><option value="warn">warn</option><option value="info">info</option><option value="debug">debug</option></select></label>
          </section>

          <section class="panel">
            <h3><BookOpen class="icon" />Zotero 连接</h3>
            <label class="field"><span>Local API</span><input type="text" v-model="config.zotero.api_base" /></label>
            <label class="field">
              <span class="label-with-help">请求超时<span class="hint-icon" title="访问 Zotero Local API 的超时时间，单位为秒。"><HelpCircle class="icon" /></span></span>
              <input type="number" v-model.number="config.zotero.request_timeout_seconds" min="1" />
            </label>
            <label class="check"><input type="checkbox" v-model="config.zotero.include_user_library" />索引个人库</label>
            <label class="field"><span>群组库</span><select v-model="config.zotero.group_mode"><option value="all">全部索引</option><option value="none">不索引</option></select></label>
          </section>

          <section class="panel">
            <h3><Search class="icon" />搜索与索引</h3>
            <label class="field"><span>默认结果数</span><input type="number" v-model.number="config.search.default_limit" min="1" max="100" /></label>
            <label class="field"><span>结果数上限</span><input type="number" v-model.number="config.search.maximum_limit" min="1" max="500" /></label>
            <label class="check"><input type="checkbox" v-model="config.search.index_abstract" />索引摘要</label>
            <label class="check"><input type="checkbox" v-model="config.search.index_extra" />索引 Extra 字段</label>
            <label class="check"><input type="checkbox" v-model="config.search.short_query_fallback" />短查询回退<span class="hint-icon" title="1-2 个字符的短词改用 LIKE 查询，改善中文短词搜索。"><HelpCircle class="icon" /></span></label>
            <label class="check"><input type="checkbox" v-model="config.search.store_raw_json" />保存原始 JSON<span class="hint-icon" title="保存 Zotero 返回的完整条目数据，供命名模板读取类型特有字段。"><HelpCircle class="icon" /></span></label>
          </section>

          <section class="panel">
            <h3><HardDrive class="icon" />存储</h3>
            <label class="field">
              <span class="label-with-help">索引数据库路径<span class="hint-icon" title="留空时使用默认位置；便携模式为 exe 旁的 data\\index.sqlite。修改数据库路径后需要重启。"><HelpCircle class="icon" /></span></span>
              <input type="text" v-model="config.storage.database" placeholder="留空使用默认位置" />
            </label>
          </section>

          <section class="panel wide">
            <h3><Link class="icon" />链接文件</h3>
            <div class="settings-section">
              <h4>目录设置</h4>
              <div class="split">
                <div class="platform-box">
                  <h5>Windows</h5>
                  <label class="check"><input type="checkbox" v-model="config.mirror.windows.enabled" />启用 .url 链接</label>
                  <label class="field"><span>链接文件目录</span><input type="text" v-model="config.mirror.windows.directory" /></label>
                </div>
                <div class="platform-box">
                  <h5>macOS</h5>
                  <label class="check"><input type="checkbox" v-model="config.mirror.macos.enabled" />启用 .webloc 链接</label>
                  <label class="field"><span>链接文件目录</span><input type="text" v-model="config.mirror.macos.directory" /></label>
                </div>
              </div>
            </div>
            <div class="settings-section">
              <h4>命名模板</h4>
              <label class="check"><input type="checkbox" v-model="followZotero" />跟随 Zotero 命名模板</label>
              <label v-if="!followZotero" class="field field-top">
                <span>自定义模板</span>
                <textarea rows="5" spellcheck="false" v-model="config.mirror.windows.template"></textarea>
              </label>
            </div>
            <div class="settings-section">
              <h4>URI 模板</h4>
              <div class="split">
                <label class="field">
                  <span class="label-with-help">Windows URI<span class="hint-icon" title="写入 .url 文件 URL= 后面的内容。默认 {select_uri}，可用占位符：{select_uri}、{item_key}、{itemKey}、{title}。"><HelpCircle class="icon" /></span></span>
                  <input type="text" v-model="config.mirror.windows.uri_template" placeholder="{select_uri}" />
                </label>
                <label class="field">
                  <span class="label-with-help">macOS URI<span class="hint-icon" title="写入 .webloc 文件的 URL。默认 {select_uri}，可用占位符：{select_uri}、{item_key}、{itemKey}、{title}。"><HelpCircle class="icon" /></span></span>
                  <input type="text" v-model="config.mirror.macos.uri_template" placeholder="{select_uri}" />
                </label>
              </div>
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
