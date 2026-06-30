// Dependency-free SVG charts. Build-free, theme-aware via CSS classes.
import { h } from "./ui.js";

const SVGNS = "http://www.w3.org/2000/svg";
function s(tag, attrs = {}, ...children) {
  const el = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) if (v != null) el.setAttribute(k, v);
  for (const c of children.flat()) if (c != null) el.appendChild(c);
  return el;
}

// Area+line chart from time buckets. `field` is a key on each bucket.
export function lineChart(buckets, field) {
  const W = 600, H = 180, padL = 8, padR = 8, padT = 12, padB = 14;
  const data = (buckets || []).map((b) => Number(b[field] || 0));
  const svg = s("svg", { class: "chart", viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: "none" });
  // baseline + mid gridlines
  for (let i = 0; i <= 2; i++) {
    const y = padT + ((H - padT - padB) * i) / 2;
    svg.appendChild(s("line", { class: "axis", x1: padL, y1: y, x2: W - padR, y2: y }));
  }
  if (data.length < 2) return svg;
  const max = Math.max(1, ...data);
  const innerW = W - padL - padR, innerH = H - padT - padB;
  const x = (i) => padL + (innerW * i) / (data.length - 1);
  const y = (v) => padT + innerH - (innerH * v) / max;
  let line = "", area = `M ${x(0)} ${y(0)}`;
  data.forEach((v, i) => {
    const cmd = i === 0 ? "M" : "L";
    line += `${cmd} ${x(i).toFixed(1)} ${y(v).toFixed(1)} `;
    area += ` L ${x(i).toFixed(1)} ${y(v).toFixed(1)}`;
  });
  area += ` L ${x(data.length - 1)} ${y(0)} Z`;
  svg.appendChild(s("path", { class: "area", d: area }));
  svg.appendChild(s("path", { class: "line", d: line }));
  return svg;
}

// Horizontal bar list. items: [{key, value, sub}]
export function barList(items, fmt) {
  const max = Math.max(1, ...items.map((i) => i.value));
  return h(
    "div",
    { class: "barlist" },
    items.length === 0 ? h("div", { class: "empty" }, "—") : null,
    ...items.map((it) =>
      h(
        "div",
        { class: "b" },
        h("div", { class: "mono", title: it.key }, it.key || "—"),
        h("div", { class: "track" }, h("i", { style: `width:${(it.value / max) * 100}%` })),
        h("div", { class: "val" }, fmt ? fmt(it.value) : it.value)
      )
    )
  );
}
