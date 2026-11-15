import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { convApi, TOOL_LABEL, type AnalyzeProgress, type Conversation, type ConversationsState, type ToolId } from "./api";

const TOOLS: ToolId[] = ["claude", "codex", "agy"];
const when = (ms: number) => {
  const d = new Date(ms);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? `hoje ${d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}`
    : d.toLocaleString("pt-BR", { day: "2-digit", month: "2-digit", year: "2-digit", hour: "2-digit", minute: "2-digit" });
};
const kb = (n: number) => (n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
/** Nome curto do projeto: última parte da pasta real (ou da chave). */
const projectName = (c: Conversation) => (c.cwd ?? c.project).split(/[\\/]|--?/).filter(Boolean).pop() ?? c.project;

type SortKey = "date" | "title" | "project" | "size" | "profiles";
type Status = "all" | "shared" | "single" | "diverged";
const PAGE_SIZES = [50, 100, 200];

const PREF_KEY = "convViewPrefs";
const readPrefs = () => {
  try { return JSON.parse(localStorage.getItem(PREF_KEY) ?? "{}"); } catch { return {}; }
};

export function ConversationsView(props: {
  initialTool: ToolId;
  installed: Record<ToolId, boolean>;
  setStatus: (s: { text: string; error?: boolean }) => void;
}) {
  const prefs = useMemo(readPrefs, []);
  const [tool, setTool] = useState<ToolId>(props.initialTool);
  const [state, setState] = useState<ConversationsState | null>(null);
  const [query, setQuery] = useState("");
  const [project, setProject] = useState("");
  const [profileFilter, setProfileFilter] = useState(""); // "in:<perfil>" | "out:<perfil>"
  const [status, setStatusFilter] = useState<Status>("all");
  const [sort, setSort] = useState<{ key: SortKey; desc: boolean }>(prefs.sort ?? { key: "date", desc: true });
  const [pageSize, setPageSize] = useState<number>(prefs.pageSize ?? 50);
  const [page, setPage] = useState(0);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const req = useRef(0);
  const fail = (e: unknown) => props.setStatus({ text: String(e), error: true });

  /** Aplica um estado novo sem perder títulos que já chegaram (o cache só é
   *  gravado no fim da análise). */
  const applyState = useCallback((next: ConversationsState) => {
    setState((prev) => {
      if (!prev || prev.tool !== next.tool) return next;
      const known = new Map(prev.conversations.filter((c) => c.title !== null).map((c) => [c.project + "/" + c.id, c]));
      return {
        ...next,
        conversations: next.conversations.map((c) => {
          const k = known.get(c.project + "/" + c.id);
          return c.title === null && k ? { ...c, title: k.title, cwd: c.cwd ?? k.cwd } : c;
        }),
      };
    });
  }, []);

  const load = useCallback(() => {
    setState(null);
    setProgress(null);
    const myReq = Date.now();
    req.current = myReq;
    convApi
      .get(tool)
      .then((s) => {
        if (req.current !== myReq) return;
        setState(s);
        if (s.supported && s.pending > 0) {
          // títulos em segundo plano: a lista já aparece, eles vão chegando
          setProgress({ done: 0, total: s.pending });
          convApi.analyze(tool, myReq).catch(fail);
        }
      })
      .catch(fail);
  }, [tool]);
  useEffect(() => { load(); }, [load]);

  useEffect(() => {
    const un = listen<AnalyzeProgress>("conversations-progress", ({ payload: p }) => {
      if (p.req !== req.current) return;
      if (p.items.length) {
        const byKey = new Map(p.items.map((d) => [d.project + "/" + d.id, d]));
        setState((s) => s && {
          ...s,
          conversations: s.conversations.map((c) => {
            const d = byKey.get(c.project + "/" + c.id);
            return d ? { ...c, title: d.title, cwd: c.cwd ?? d.cwd } : c;
          }),
        });
      }
      setProgress((pr) => (p.finished ? null : { done: p.done, total: pr?.total ?? p.done }));
    });
    return () => { un.then((f) => f()); };
  }, []);

  // ao trocar de ferramenta, filtros que dependem dela deixam de valer
  useEffect(() => { setProject(""); setProfileFilter(""); setPage(0); }, [tool]);
  // qualquer mudança de filtro volta para a primeira página
  useEffect(() => { setPage(0); }, [query, project, profileFilter, status, pageSize]);
  useEffect(() => {
    try { localStorage.setItem(PREF_KEY, JSON.stringify({ sort, pageSize })); } catch { /* sem storage */ }
  }, [sort, pageSize]);

  const toggle = async (c: Conversation, profile: string, on: boolean) => {
    try {
      applyState(await convApi.set(tool, c.project, c.id, profile, on));
      props.setStatus({ text: on ? `Conversa ligada ao perfil "${profile}"` : `Conversa removida do perfil "${profile}"` });
    } catch (e) { fail(e); }
  };

  const resync = async (c: Conversation) => {
    try {
      applyState(await convApi.resync(tool, c.project, c.id));
      props.setStatus({ text: "Cópias juntadas na versão mais recente (as antigas ficaram como .bak)" });
    } catch (e) { fail(e); }
  };

  const projects = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of state?.conversations ?? []) m.set(c.project, projectName(c));
    return [...m.entries()].sort((a, b) => a[1].localeCompare(b[1], "pt-BR"));
  }, [state]);

  const present = (c: Conversation) => c.profiles.filter((p) => p.state !== "off").length;

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const [mode, prof] = profileFilter ? profileFilter.split(/:(.*)/s) : ["", ""];
    const list = (state?.conversations ?? []).filter((c) => {
      if (project && c.project !== project) return false;
      if (prof) {
        const st = c.profiles.find((p) => p.profile === prof)?.state ?? "off";
        if (mode === "in" && st === "off") return false;
        if (mode === "out" && st !== "off") return false;
      }
      const n = present(c);
      if (status === "shared" && n < 2) return false;
      if (status === "single" && n !== 1) return false;
      if (status === "diverged" && !c.profiles.some((p) => p.state === "diverged")) return false;
      if (q && !((c.title ?? "").toLowerCase().includes(q) || (c.cwd ?? c.project).toLowerCase().includes(q) || c.id.toLowerCase().startsWith(q)))
        return false;
      return true;
    });
    const dir = sort.desc ? -1 : 1;
    const cmp: Record<SortKey, (a: Conversation, b: Conversation) => number> = {
      date: (a, b) => a.updatedAt - b.updatedAt,
      title: (a, b) =>
        a.title === null ? (b.title === null ? 0 : 1) : b.title === null ? -1 : a.title.localeCompare(b.title, "pt-BR"),
      project: (a, b) => projectName(a).localeCompare(projectName(b), "pt-BR") || b.updatedAt - a.updatedAt,
      size: (a, b) => a.size - b.size,
      profiles: (a, b) => present(a) - present(b) || b.updatedAt - a.updatedAt,
    };
    return [...list].sort((a, b) => dir * cmp[sort.key](a, b));
  }, [state, query, project, profileFilter, status, sort]);

  const pages = Math.max(1, Math.ceil(filtered.length / pageSize));
  const current = Math.min(page, pages - 1);
  const rows = filtered.slice(current * pageSize, current * pageSize + pageSize);
  const anyFilter = query || project || profileFilter || status !== "all";

  const sortBy = (key: SortKey) =>
    setSort((s) => (s.key === key ? { key, desc: !s.desc } : { key, desc: key === "date" || key === "size" || key === "profiles" }));
  const SortTh = (p: { k: SortKey; label: string; className?: string }) => (
    <th className={"sortable " + (p.className ?? "")} onClick={() => sortBy(p.k)} title="Ordenar">
      {p.label}
      <span className="sort-ind">{sort.key === p.k ? (sort.desc ? "▼" : "▲") : ""}</span>
    </th>
  );

  return (
    <div className="page wide">
      <div className="page-head">
        <div>
          <h2>Conversas</h2>
          <p className="sub">
            Marque em quais perfis cada conversa aparece. É o <b>mesmo arquivo</b> em todos: continuar em qualquer conta
            atualiza as outras, e ela aparece no <code>/resume</code> de cada perfil marcado.
          </p>
        </div>
        <div className="detail-actions">
          <button className="ghost" onClick={load}>Atualizar</button>
        </div>
      </div>

      <div className="tabs flat" role="tablist">
        {TOOLS.map((t) => (
          <button key={t} role="tab" aria-selected={t === tool}
            className={"tab" + (t === tool ? " active" : "") + (props.installed[t] ? "" : " missing")}
            onClick={() => setTool(t)}>
            <span className={"mark mark-" + t}>{t === "claude" ? "C" : t === "codex" ? "X" : "A"}</span>
            <span>{TOOL_LABEL[t]}</span>
          </button>
        ))}
      </div>

      {!state && <p className="sub">Carregando…</p>}
      {state && !state.supported && <div className="banner">{state.note}</div>}

      {state?.supported && (
        <>
          {state.profiles.length < 2 && (
            <div className="banner">Crie pelo menos mais um perfil de {TOOL_LABEL[tool]} para sincronizar conversas entre contas.</div>
          )}

          {progress && (
            <div className="progress" title="Os títulos ficam em cache: da próxima vez só as conversas novas são analisadas">
              <div className="progress-bar" style={{ width: `${Math.round((progress.done / Math.max(1, progress.total)) * 100)}%` }} />
              <span>Analisando títulos… {progress.done} de {progress.total}</span>
            </div>
          )}

          <div className="filters">
            <input className="search grow" placeholder="Buscar por título, pasta ou ID…" value={query}
              onChange={(e) => setQuery(e.target.value)} />
            <select value={project} onChange={(e) => setProject(e.target.value)} title="Projeto">
              <option value="">Todos os projetos ({projects.length})</option>
              {projects.map(([key, name]) => <option key={key} value={key}>{name}</option>)}
            </select>
            <select value={profileFilter} onChange={(e) => setProfileFilter(e.target.value)} title="Perfil">
              <option value="">Qualquer perfil</option>
              {state.profiles.map((p) => <option key={"in" + p} value={`in:${p}`}>Está em: {p}</option>)}
              {state.profiles.map((p) => <option key={"out" + p} value={`out:${p}`}>Não está em: {p}</option>)}
            </select>
            <select value={status} onChange={(e) => setStatusFilter(e.target.value as Status)} title="Situação">
              <option value="all">Todas</option>
              <option value="shared">Em sync (2+ perfis)</option>
              <option value="single">Só em um perfil</option>
              <option value="diverged">Fora de sync</option>
            </select>
            {anyFilter && (
              <button className="small ghost"
                onClick={() => { setQuery(""); setProject(""); setProfileFilter(""); setStatusFilter("all"); }}>
                Limpar filtros
              </button>
            )}
          </div>

          <table className="grid">
            <thead>
              <tr>
                <SortTh k="title" label="Conversa" />
                <SortTh k="project" label="Projeto" />
                <SortTh k="date" label="Atualizada" className="nowrap" />
                <SortTh k="size" label="Tamanho" className="right" />
                {state.profiles.map((p) => <th key={p} className="center">{p}</th>)}
                <SortTh k="profiles" label="" />
              </tr>
            </thead>
            <tbody>
              {rows.length === 0 && (
                <tr><td colSpan={state.profiles.length + 5} className="empty">Nenhuma conversa com esses filtros.</td></tr>
              )}
              {rows.map((c) => {
                const diverged = c.profiles.some((p) => p.state === "diverged");
                return (
                  <tr key={c.project + c.id}>
                    <td className="conv-cell">
                      {c.title === null
                        ? <div className="conv-title loading-text">analisando…</div>
                        : <div className="conv-title" title={c.title}>{c.title}</div>}
                      <div className="skill-desc"><code>{c.id.slice(0, 8)}</code></div>
                    </td>
                    <td className="nowrap" title={c.cwd ?? c.project}>{projectName(c)}</td>
                    <td className="nowrap">{when(c.updatedAt)}</td>
                    <td className="right nowrap">{kb(c.size)}</td>
                    {c.profiles.map((p) => (
                      <td key={p.profile} className="center">
                        {p.state === "diverged" ? (
                          <span className="tone warn" title="Este perfil tem uma cópia separada da conversa">fora de sync</span>
                        ) : (
                          <input type="checkbox" className="check" checked={p.state === "on"}
                            onChange={(e) => toggle(c, p.profile, e.target.checked)}
                            title={p.state === "on" ? `Tirar do perfil ${p.profile}` : `Mostrar no perfil ${p.profile}`} />
                        )}
                      </td>
                    ))}
                    <td className="right">
                      {diverged && <button className="small" onClick={() => resync(c)}>Sincronizar</button>}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>

          <div className="pager">
            <span className="muted-inline">
              {filtered.length === 0 ? "0 conversas" :
                `${current * pageSize + 1}–${Math.min(filtered.length, (current + 1) * pageSize)} de ${filtered.length}` +
                (filtered.length !== state.conversations.length ? ` (de ${state.conversations.length} no total)` : "")}
            </span>
            <span className="pager-nav">
              <button className="small ghost" disabled={current === 0} onClick={() => setPage(0)} title="Primeira">«</button>
              <button className="small ghost" disabled={current === 0} onClick={() => setPage(current - 1)}>‹ Anterior</button>
              <span className="page-num">Página {current + 1} de {pages}</span>
              <button className="small ghost" disabled={current >= pages - 1} onClick={() => setPage(current + 1)}>Próxima ›</button>
              <button className="small ghost" disabled={current >= pages - 1} onClick={() => setPage(pages - 1)} title="Última">»</button>
              <select value={pageSize} onChange={(e) => setPageSize(Number(e.target.value))} title="Por página">
                {PAGE_SIZES.map((n) => <option key={n} value={n}>{n} por página</option>)}
              </select>
            </span>
          </div>

          <p className="hint">
            Evite continuar a <b>mesma</b> conversa em duas contas ao mesmo tempo: as mensagens se misturariam no arquivo.
          </p>
        </>
      )}
    </div>
  );
}
