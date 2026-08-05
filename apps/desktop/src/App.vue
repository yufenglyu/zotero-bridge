<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";

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
    windows: { enabled: boolean; directory: string; template: string };
    macos: { enabled: boolean; directory: string; template: string };
  };
  maintenance: { optimize_after_updates: number; retain_logs_days: number };
  storage: { database: string };
}

type ThemeMode = "light" | "dark" | "system";

const THEME_KEY = "zsb-theme-mode";
const themeMode = ref<ThemeMode>((localStorage.getItem(THEME_KEY) as ThemeMode) || "system");
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

let timer: number | undefined;
let themeTimer: number | undefined;
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
const onSystemTheme = (e: MediaQueryListEvent) => (systemDark.value = e.matches);

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

async function openDir(which: string) {
  await safeInvoke("open_dir", { which });
}

async function loadConfig() {
  const c = await safeInvoke<Config>("get_config");
  if (c) {
    config.value = c;
    savedSnapshot.value = JSON.stringify(c);
  }
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
  // 跟随系统模式下轮询系统主题（部分 Windows 环境 change 事件不可靠）
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
  <div class="app">
    <header class="topbar">
      <div class="brand">
        <span class="logo">Z</span>
        <div>
          <h1>Zotero Search Bridge</h1>
          <p class="subtitle">本地文献检索桥</p>
        </div>
      </div>
      <div class="topbar-right">
        <div class="sync-state" v-if="status">
          <span
            class="dot"
            :class="status.paused ? 'dot-paused' : status.zotero_running ? 'dot-ok' : 'dot-off'"
          ></span>
          <span v-if="status.paused">已暂停</span>
          <span v-else-if="status.zotero_running">同步中</span>
          <span v-else>等待 Zotero</span>
        </div>
        <div class="seg seg-sm" role="radiogroup" aria-label="主题">
          <button :class="{ active: themeMode === 'light' }" @click="themeMode = 'light'">
            浅色
          </button>
          <button :class="{ active: themeMode === 'dark' }" @click="themeMode = 'dark'">
            深色
          </button>
          <button :class="{ active: themeMode === 'system' }" @click="themeMode = 'system'">
            跟随系统
          </button>
        </div>
      </div>
    </header>

    <nav class="tabs">
      <button :class="{ active: tab === 'status' }" @click="tab = 'status'">状态</button>
      <button :class="{ active: tab === 'settings' }" @click="tab = 'settings'">
        设置<span v-if="dirty" class="dirty-dot" title="有未保存的修改"></span>
      </button>
      <button :class="{ active: tab === 'doctor' }" @click="tab = 'doctor'">诊断</button>
    </nav>

    <p v-if="!tauriAvailable" class="banner">
      未检测到 Tauri 运行环境：浏览器预览模式下数据不可用，请在桌面程序中查看。
    </p>
    <p v-if="message" class="banner banner-ok" @click="message = ''">
      {{ message }}<span class="banner-close">×</span>
    </p>

    <!-- 状态页 -->
    <section v-if="tab === 'status' && status" class="page">
      <div class="cards">
        <div class="card">
          <div class="card-value">{{ status.item_count.toLocaleString() }}</div>
          <div class="card-label">已索引条目</div>
        </div>
        <div class="card">
          <div class="card-value">{{ status.library_count }}</div>
          <div class="card-label">文献库</div>
        </div>
        <div class="card">
          <div class="card-value">{{ status.pending_jobs }}</div>
          <div class="card-label">待执行镜像任务</div>
        </div>
        <div class="card" :class="{ warn: status.failed_jobs > 0 }">
          <div class="card-value">{{ status.failed_jobs }}</div>
          <div class="card-label">失败任务</div>
        </div>
      </div>

      <div class="panel">
        <h3>同步控制</h3>
        <div class="row">
          <span>
            当前实例
            <i
              class="help"
              title="当前索引绑定的 Zotero 程序标识（Server ID）。换电脑、重装 Zotero 或切换 profile 后该标识会变化，程序会自动检测到实例变更并重新全量同步，避免不同 Zotero 的数据混在一起。"
              >?</i
            >
          </span>
          <b>{{ status.instance || "未知" }}</b>
        </div>
        <div class="row"><span>最近同步</span><b>{{ fmtTime(status.last_sync_at) }}</b></div>
        <div class="panel-actions">
          <button class="primary" :disabled="busy" @click="syncNow">立即同步</button>
          <button :disabled="busy" @click="togglePause">
            {{ status.paused ? "恢复同步" : "暂停同步" }}
          </button>
          <button class="danger" :disabled="busy" @click="rebuildIndex">重建索引</button>
        </div>
      </div>

      <div class="panel">
        <h3>文献库</h3>
        <div v-for="lib in status.libraries" :key="lib.kind + lib.zotero_library_id" class="lib">
          <span class="tag">{{ lib.kind }}</span>
          <span class="lib-name">{{ lib.display_name }}</span>
          <span class="lib-meta">id={{ lib.zotero_library_id }} · 版本 {{ lib.last_version }}</span>
          <span class="lib-state" :class="{ off: !lib.enabled }">
            {{ lib.enabled ? "启用" : "停用" }}
          </span>
          <p v-if="lib.last_error" class="lib-error">{{ lib.last_error }}</p>
        </div>
      </div>

      <div class="panel">
        <h3>目录与日志</h3>
        <div class="link-row" @click="openDir('mirror')">
          <span>镜像目录</span><em>Listary 索引此目录以定位文献</em>
        </div>
        <div class="link-row" @click="openDir('config')">
          <span>配置目录</span><em>config.toml 所在位置</em>
        </div>
        <div class="link-row" @click="openDir('logs')">
          <span>日志目录</span><em>排查同步问题时查看</em>
        </div>
      </div>
    </section>

    <!-- 设置页 -->
    <section v-if="tab === 'settings' && config" class="page">
      <div class="panel">
        <h3>外观</h3>
        <div class="seg" role="radiogroup" aria-label="主题">
          <button :class="{ active: themeMode === 'light' }" @click="themeMode = 'light'">
            浅色
          </button>
          <button :class="{ active: themeMode === 'dark' }" @click="themeMode = 'dark'">
            深色
          </button>
          <button :class="{ active: themeMode === 'system' }" @click="themeMode = 'system'">
            跟随系统
          </button>
        </div>
        <p class="hint">“跟随系统”会随 Windows 深/浅色自动切换，并每秒轮询一次系统状态。</p>
      </div>

      <div class="panel">
        <h3>常规</h3>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.app.start_at_login" />
          <span>开机自动启动</span>
        </label>
        <label class="field">
          <span>轮询周期（秒）</span>
          <input type="number" v-model.number="config.app.poll_interval_seconds" min="5" />
        </label>
        <label class="field">
          <span>日志级别</span>
          <select v-model="config.app.log_level">
            <option value="error">error</option>
            <option value="warn">warn</option>
            <option value="info">info</option>
            <option value="debug">debug</option>
          </select>
        </label>
      </div>

      <div class="panel">
        <h3>Zotero 连接</h3>
        <label class="field">
          <span>Local API 地址</span>
          <input type="text" v-model="config.zotero.api_base" />
        </label>
        <label class="field">
          <span>请求超时（秒）</span>
          <input type="number" v-model.number="config.zotero.request_timeout_seconds" min="1" />
        </label>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.zotero.include_user_library" />
          <span>索引个人库</span>
        </label>
        <label class="field">
          <span>群组库</span>
          <select v-model="config.zotero.group_mode">
            <option value="all">全部索引</option>
            <option value="none">不索引</option>
          </select>
        </label>
      </div>

      <div class="panel">
        <h3>搜索与索引</h3>
        <label class="field">
          <span>默认结果数</span>
          <input type="number" v-model.number="config.search.default_limit" min="1" max="100" />
        </label>
        <label class="field">
          <span>结果数上限</span>
          <input type="number" v-model.number="config.search.maximum_limit" min="1" max="500" />
        </label>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.search.index_abstract" />
          <span>索引摘要</span>
        </label>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.search.index_extra" />
          <span>索引 Extra 字段</span>
        </label>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.search.short_query_fallback" />
          <span>短查询回退（单字也走 LIKE 兜底）</span>
        </label>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.search.store_raw_json" />
          <span>保存原始 JSON（便于调试/迁移）</span>
        </label>
      </div>

      <div class="panel">
        <h3>镜像（外部工具定位）</h3>
        <h4>Windows · .url（Listary）</h4>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.mirror.windows.enabled" />
          <span>启用 .url 镜像</span>
        </label>
        <label class="field">
          <span>镜像目录</span>
          <input type="text" v-model="config.mirror.windows.directory" />
        </label>
        <label class="field">
          <span>文件名模板</span>
          <input type="text" v-model="config.mirror.windows.template" />
        </label>
        <h4>macOS · .webloc</h4>
        <label class="field checkbox">
          <input type="checkbox" v-model="config.mirror.macos.enabled" />
          <span>启用 .webloc 镜像</span>
        </label>
        <label class="field">
          <span>镜像目录</span>
          <input type="text" v-model="config.mirror.macos.directory" />
        </label>
      </div>

      <div class="panel">
        <h3>存储</h3>
        <label class="field">
          <span>索引数据库路径</span>
          <input type="text" v-model="config.storage.database" placeholder="留空使用默认位置" />
        </label>
        <p class="hint">
          留空 = 默认位置（便携模式为 exe 旁 data\）；支持 %VAR% 和 ~。
          修改后需重启程序生效，已有索引不会自动迁移。
        </p>
      </div>

      <div class="save-bar">
        <span v-if="dirty" class="save-hint">有未保存的修改</span>
        <span v-else class="save-hint ok">设置已保存</span>
        <button class="primary" :disabled="busy || !dirty" @click="saveConfig">保存设置</button>
      </div>
    </section>

    <!-- 诊断页 -->
    <section v-if="tab === 'doctor'" class="page">
      <div class="panel">
        <h3>环境诊断</h3>
        <p class="hint">
          检查 Zotero 本地 API 连通性、数据库与镜像目录可写性。同步异常时先运行诊断。
        </p>
        <div class="panel-actions">
          <button class="primary" :disabled="busy" @click="runDoctor">运行诊断</button>
        </div>
      </div>
      <div class="panel" v-if="doctorLines.length">
        <pre class="doctor">{{ doctorLines.join("\n") }}</pre>
      </div>
    </section>
  </div>
</template>
