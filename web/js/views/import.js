// Import a HuggingFace model → convert to a local .cmf (our format) with a
// live progress view. Search HF, pick quantization + advanced params, watch
// the conversion (multiple in parallel, resumable across tab switches), then
// register the result as a local model.
import { h, mount, toast } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";

const QUANTS = [
  ["Q8_2F", "import.q.q8_2f"],
  ["Q8_ROW", "import.q.q8_row"],
  ["Q4_BLOCK", "import.q.q4"],
  ["VBIT", "import.q.vbit"],
  ["F16", "import.q.f16"],
  ["F32", "import.q.f32"],
];

// A small "?" badge with a hover/focus tooltip (same style as Settings).
function help(text) {
  return text ? h("span", { class: "help", title: text, tabindex: "0", role: "img", "aria-label": text }, "?") : null;
}
function field(label, control, hint, helpText) {
  return h("label", { class: "field" },
    h("span", {}, label, helpText ? " " : null, help(helpText)),
    control,
    hint ? h("div", { class: "hint" }, hint) : null);
}
function opt(v, label, sel) {
  return h("option", { value: v, selected: sel ? true : null }, label || v);
}
function fmtNum(n) {
  if (n == null) return "—";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "k";
  return String(n);
}
function fmtBytes(n) {
  if (!n) return "";
  const g = n / 1e9;
  return g >= 1 ? g.toFixed(2) + " GB" : (n / 1e6).toFixed(0) + " MB";
}
// state → localized label (falls back to the raw state)
function stateLabel(s) {
  const k = "import.state." + s;
  const v = t(k);
  return v === k ? s : v;
}

export function renderImport() {
  let selected = null;
  let lastModels = []; // cache of the last search, so re-highlighting needs no refetch
  let jobsTimer = null;

  const searchIn = h("input", {
    class: "hf-search",
    placeholder: t("import.searchPlaceholder"),
    autofocus: true,
  });
  const results = h("div", { class: "hf-grid" });
  const formHost = h("div", {});
  const jobsHost = h("div", {});

  // ── live HF search (debounced) ──
  let debounce = null;
  function renderCards(models) {
    if (!models.length) {
      mount(results, h("div", { class: "muted pad" }, t("import.noResults")));
      return;
    }
    mount(results, ...models.map(modelCard));
  }
  async function doSearch() {
    const q = searchIn.value.trim();
    mount(results, h("div", { class: "muted pad" }, t("import.searching")));
    try {
      const r = await api.hfSearch(q, 24);
      lastModels = r.models || [];
      renderCards(lastModels);
    } catch (e) {
      mount(results, h("div", { class: "err pad" }, "HF: " + e.message));
    }
  }
  searchIn.addEventListener("input", () => {
    clearTimeout(debounce);
    debounce = setTimeout(doSearch, 300);
  });

  function modelCard(m) {
    const card = h(
      "div",
      { class: "hf-card" + (selected && selected.id === m.id ? " sel" : "") },
      h("div", { class: "hf-id" }, m.id),
      h(
        "div",
        { class: "hf-meta" },
        m.pipeline_tag ? h("span", { class: "badge" }, m.pipeline_tag) : null,
        h("span", { class: "muted" }, "▼ " + fmtNum(m.downloads)),
        h("span", { class: "muted" }, "♥ " + fmtNum(m.likes)),
        (m.tags || []).some((x) => x === "gated") ? h("span", { class: "badge bad" }, "gated") : null
      )
    );
    card.addEventListener("click", () => selectModel(m));
    return card;
  }

  function selectModel(m) {
    selected = m;
    renderCards(lastModels); // re-highlight from cache — no refetch, no flash
    renderForm();
    formHost.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }

  // ── conversion form ──
  function renderForm() {
    if (!selected) {
      mount(formHost);
      return;
    }
    const nameIn = h("input", { value: (selected.id.split("/").pop() || "model").toLowerCase() });
    const quantSel = h("select", {}, ...QUANTS.map(([v], i) => opt(v, v, i === 0)));
    const quantHint = h("div", { class: "hint" }, t("import.q.q8_2f"));
    quantSel.addEventListener("change", () => {
      const k = QUANTS.find(([v]) => v === quantSel.value);
      quantHint.textContent = k ? t(k[1]) : "";
    });

    // advanced (collapsed)
    const linearSel = h("select", {}, opt("", t("import.adv.auto")), opt("gated_delta_net", "gated_delta_net"), opt("vmf_phase", "vmf_phase"));
    const nphaseIn = h("input", { type: "number", min: "1", placeholder: "16" });
    const vbitSel = h("select", {}, opt("", t("import.adv.auto")), opt("log2", "log2"), opt("cubic", "cubic"));
    const meanBitsIn = h("input", { type: "number", step: "0.1", placeholder: "5.0" });
    const shardIn = h("input", { type: "number", step: "0.5", placeholder: t("import.adv.noShard") });
    const skipMtp = h("input", { type: "checkbox" });
    // O(1) attention (cortiq ≥ 0.2.0): constant-memory streaming attention hint,
    // weights byte-identical — the runtime applies it at load.
    const o1Sel = h("select", {},
      opt("", t("import.adv.o1Off")),
      opt("all", t("import.adv.o1All")),
      opt("deep", t("import.adv.o1Deep")),
      opt("custom", t("import.adv.o1Custom")));
    const o1DeepIn = h("input", { type: "number", min: "1", placeholder: "12" });
    const o1CustomIn = h("input", { placeholder: "0,4,8" });
    const o1M = h("input", { type: "number", min: "1", placeholder: "32" });
    const o1Window = h("input", { type: "number", min: "0", placeholder: "128" });
    const o1Sink = h("input", { type: "number", min: "0", placeholder: "4" });
    const o1SpecRow = h("div", { class: "row", style: "display:none" },
      field(t("import.adv.o1DeepN"), o1DeepIn),
      field(t("import.adv.o1Layers"), o1CustomIn));
    const o1Knobs = h("div", { class: "row", style: "display:none" },
      field(t("import.adv.o1mLabel"), o1M, t("import.adv.o1mHint"), t("import.adv.o1mHelp")),
      field(t("import.adv.o1windowLabel"), o1Window, t("import.adv.o1windowHint"), t("import.adv.o1windowHelp")),
      field(t("import.adv.o1sinkLabel"), o1Sink, t("import.adv.o1sinkHint"), t("import.adv.o1sinkHelp")));
    o1Sel.addEventListener("change", () => {
      const on = !!o1Sel.value;
      o1Knobs.style.display = on ? "" : "none";
      o1SpecRow.style.display = o1Sel.value === "deep" || o1Sel.value === "custom" ? "" : "none";
      o1DeepIn.parentElement.style.display = o1Sel.value === "deep" ? "" : "none";
      o1CustomIn.parentElement.style.display = o1Sel.value === "custom" ? "" : "none";
    });
    const advBody = h(
      "div",
      { class: "adv-body", style: "display:none" },
      h("div", { class: "row" },
        field(t("import.adv.linearCore"), linearSel, t("import.adv.linearHint")),
        field(t("import.adv.nphase"), nphaseIn)),
      h("div", { class: "row" },
        field(t("import.adv.vbitShape"), vbitSel),
        field(t("import.adv.meanBits"), meanBitsIn)),
      h("div", { class: "row" },
        field(t("import.adv.shard"), shardIn),
        field("MTP", h("label", { class: "check" }, skipMtp, t("import.adv.skipMtp")))),
      field(t("import.adv.o1"), o1Sel, t("import.adv.o1Hint")),
      o1SpecRow,
      o1Knobs,
    );
    const advToggle = h("button", { class: "btn ghost sm" }, "⚙ " + t("import.adv.title"));
    advToggle.addEventListener("click", () => {
      advBody.style.display = advBody.style.display === "none" ? "block" : "none";
    });

    const goBtn = h("button", { class: "btn primary" }, "⬇ " + t("import.convert"));
    goBtn.addEventListener("click", async () => {
      const params = {
        repo: selected.id,
        quant: quantSel.value,
        name: nameIn.value.trim(),
        skip_mtp: skipMtp.checked,
      };
      if (linearSel.value) params.linear_core = linearSel.value;
      if (nphaseIn.value) params.nphase = parseInt(nphaseIn.value);
      if (vbitSel.value) params.vbit_shape = vbitSel.value;
      if (meanBitsIn.value) params.mean_bits = parseFloat(meanBitsIn.value);
      if (shardIn.value) params.shard_max_gb = parseFloat(shardIn.value);
      if (o1Sel.value) {
        // spec: all | deepN | explicit layer list
        params.o1 =
          o1Sel.value === "deep" ? "deep" + (parseInt(o1DeepIn.value) || 12)
          : o1Sel.value === "custom" ? o1CustomIn.value.trim()
          : "all";
        if (!params.o1) delete params.o1;
        else {
          if (o1M.value) params.o1_m = parseInt(o1M.value);
          if (o1Window.value) params.o1_window = parseInt(o1Window.value);
          if (o1Sink.value) params.o1_sink = parseInt(o1Sink.value);
        }
      }
      goBtn.disabled = true;
      try {
        await api.startImport(params);
        toast(t("import.started"), "ok");
        await refreshJobs(); // picks up the new job + (re)starts polling
        jobsHost.scrollIntoView({ behavior: "smooth", block: "nearest" });
      } catch (e) {
        toast(e.message, "err");
      }
      goBtn.disabled = false;
    });

    mount(
      formHost,
      h("div", { class: "card import-form" },
        h("div", { class: "card-title" }, t("import.configure") + ": " + selected.id),
        h("div", { class: "row" },
          field(t("import.outName"), nameIn, t("import.outNameHint")),
          field(t("import.quant"), quantSel)),
        quantHint,
        h("div", { class: "adv" }, advToggle, advBody),
        h("div", { class: "actions" }, goBtn),
      )
    );
  }

  // ── jobs panel: all conversions, live, resumable, cancellable ──
  function ensurePolling() {
    if (!jobsTimer) {
      jobsTimer = setInterval(() => {
        // stop once this view instance was replaced (navigated away)
        if (!root.isConnected) {
          stopPolling();
          return;
        }
        refreshJobs();
      }, 1500);
    }
  }
  function stopPolling() {
    if (jobsTimer) {
      clearInterval(jobsTimer);
      jobsTimer = null;
    }
  }
  // Load + render all jobs. Runs on mount (before `root` is attached — so no
  // isConnected guard here) and on each poll tick.
  async function refreshJobs() {
    let jobs = [];
    try {
      jobs = (await api.listImports()).jobs || [];
    } catch {
      return;
    }
    jobs = jobs.filter((j) => j.state !== "cancelled"); // hide user-cancelled jobs
    renderJobs(jobs);
    if (jobs.some((j) => j.state === "running")) ensurePolling();
    else stopPolling();
  }

  function jobCard(j) {
    const running = j.state === "running";
    const pct = j.progress != null ? Math.round(j.progress * 100) : null;
    // det = solid bar at a known %, run = indeterminate slider, ok/err = terminal.
    let bar;
    if (j.state === "done") {
      bar = h("div", { class: "pbar-fill ok", style: "width:100%" });
    } else if (running && pct != null) {
      bar = h("div", { class: "pbar-fill det", style: `width:${pct}%` });
    } else if (running) {
      bar = h("div", { class: "pbar-fill run" }); // CSS drives width + animation
    } else {
      bar = h("div", { class: "pbar-fill err", style: "width:100%" }); // error / cancelled
    }
    const barWrap = h("div", { class: "pbar" }, bar);
    const badge = h(
      "span",
      { class: "badge " + (j.state === "done" ? "" : running ? "" : "bad") },
      stateLabel(j.state) +
        (running && pct != null ? " · " + pct + "%" : "") +
        (j.size_bytes ? " · " + fmtBytes(j.size_bytes) : "")
    );
    const phase = h("div", { class: "muted small" }, j.phase || "");
    const logEl = h("pre", { class: "conv-log" }, (j.log || []).slice(-10).join("\n"));

    const actions = [];
    if (running) {
      const c = h("button", { class: "btn ghost sm" }, "✕ " + t("import.cancel"));
      c.addEventListener("click", async () => {
        c.disabled = true;
        try {
          await api.cancelImport(j.id);
          toast(t("import.cancelling"), "warn");
          refreshJobs();
        } catch (e) {
          toast(e.message, "err");
          c.disabled = false;
        }
      });
      actions.push(c);
    }
    if (j.state === "done") {
      const r = h("button", { class: "btn primary sm" }, "✓ " + t("import.register"));
      r.addEventListener("click", async () => {
        r.disabled = true;
        try {
          const rr = await api.registerImport(j.id);
          toast(t("import.registered", { id: rr.model_id }), "ok");
        } catch (e) {
          toast(e.message, "err");
          r.disabled = false;
        }
      });
      actions.push(r);
    }
    if (!running) {
      // delete the job + its converted .cmf file(s) from disk
      const d = h("button", { class: "btn ghost sm danger" }, "🗑 " + t("import.delete"));
      d.addEventListener("click", async () => {
        if (!window.confirm(t("import.deleteConfirm"))) return;
        d.disabled = true;
        try {
          await api.deleteImport(j.id);
          toast(t("import.deleted"), "ok");
          refreshJobs();
        } catch (e) {
          toast(e.message, "err");
          d.disabled = false;
        }
      });
      actions.push(d);
    }

    return h(
      "div",
      { class: "job" },
      h("div", { class: "job-head" }, h("b", {}, j.repo || j.id), badge),
      barWrap,
      phase,
      logEl,
      actions.length ? h("div", { class: "actions" }, ...actions) : null
    );
  }

  function renderJobs(jobs) {
    if (!jobs.length) {
      mount(jobsHost);
      return;
    }
    mount(
      jobsHost,
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-title" }, t("import.jobs")),
        ...jobs.map(jobCard)
      )
    );
  }

  const root = h(
    "div",
    {},
    h("div", { class: "view-head" },
      h("h2", {}, t("nav.import")),
      h("div", { class: "muted" }, t("import.subtitle"))),
    jobsHost, // progress panel on top, above the search
    h("div", { class: "card" }, searchIn, results),
    formHost
  );
  doSearch(); // initial trending list (mounts into `results`)
  refreshJobs(); // resume: show any running/recent conversions immediately
  return root;
}
