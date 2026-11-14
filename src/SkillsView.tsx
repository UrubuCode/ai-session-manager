import { useCallback, useEffect, useState } from "react";
import { skillsApi, TOOL_LABEL, type SkillsState, type ToolId } from "./api";
import type { DialogRequest } from "./Dialog";

const TOOLS: ToolId[] = ["claude", "codex", "agy"];

export function SkillsView(props: {
  installed: Record<ToolId, boolean>;
  ask: (r: Omit<DialogRequest, "resolve">) => Promise<string | null>;
  setStatus: (s: { text: string; error?: boolean }) => void;
}) {
  const [state, setState] = useState<SkillsState | null>(null);
  const [filter, setFilter] = useState("");
  const fail = (e: unknown) => props.setStatus({ text: String(e), error: true });

  const load = useCallback(() => skillsApi.get().then(setState).catch(fail), []);
  useEffect(() => { load(); }, [load]);

  const toggle = async (name: string, tool: ToolId, on: boolean) => {
    try {
      setState(await skillsApi.setLink(name, tool, on));
      props.setStatus({ text: `"${name}" ${on ? "ligada" : "desligada"} no ${TOOL_LABEL[tool]}` });
    } catch (e) { fail(e); }
  };

  const makeGlobal = async (tool: ToolId, name: string) => {
    const ok = await props.ask({
      message: `Tornar "${name}" global? A pasta sai do ${TOOL_LABEL[tool]} e vai para a pasta de skills globais, deixando um link no lugar — para o ${TOOL_LABEL[tool]} nada muda. Depois você pode ligar a skill nas outras ferramentas.`,
      okLabel: "Tornar global",
    });
    if (ok === null) return;
    try {
      setState(await skillsApi.makeGlobal(tool, name));
      props.setStatus({ text: `"${name}" agora é global` });
    } catch (e) { fail(e); }
  };

  const create = async () => {
    const name = await props.ask({ message: "Nome da nova skill global (ex.: revisar-pr):", input: "", okLabel: "Continuar" });
    if (!name) return;
    const desc = await props.ask({
      message: "Descrição: quando o agente deve usar esta skill? (ajuda o agente a decidir sozinho)",
      input: "", okLabel: "Criar",
    });
    if (desc === null) return;
    try {
      setState(await skillsApi.create(name, desc));
      props.setStatus({ text: `Skill criada — edite o SKILL.md na pasta que abriu e ligue nas ferramentas` });
    } catch (e) { fail(e); }
  };

  const q = filter.trim().toLowerCase();
  const match = (n: string, d: string | null) => !q || n.toLowerCase().includes(q) || (d ?? "").toLowerCase().includes(q);
  const global = state?.global.filter((s) => match(s.name, s.description)) ?? [];
  const local = state?.local.filter((s) => match(s.name, s.description)) ?? [];

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h2>Skills globais</h2>
          <p className="sub">Uma skill, várias ferramentas: ligue cada skill no Claude Code, Codex e Antigravity. Vale para todos os perfis.</p>
        </div>
        <div className="detail-actions">
          <input className="search" placeholder="Filtrar…" value={filter} onChange={(e) => setFilter(e.target.value)} />
          <button className="ghost" onClick={() => skillsApi.openFolder().catch(fail)}>Abrir pasta</button>
          <button onClick={create}>+ Nova skill</button>
        </div>
      </div>

      <table className="grid">
        <thead>
          <tr>
            <th>Skill</th>
            {TOOLS.map((t) => (
              <th key={t} className="center" title={props.installed[t] ? "" : "não instalado"}>
                <span className={"mark mark-" + t}>{t === "claude" ? "C" : t === "codex" ? "X" : "A"}</span>
                <span className="th-label">{TOOL_LABEL[t]}</span>
              </th>
            ))}
            <th />
          </tr>
        </thead>
        <tbody>
          {global.length === 0 && (
            <tr><td colSpan={5} className="empty">Nenhuma skill global ainda. Crie uma ou torne global uma skill existente abaixo.</td></tr>
          )}
          {global.map((s) => (
            <tr key={s.name}>
              <td>
                <div className="skill-name">{s.name}</div>
                {s.description && <div className="skill-desc">{s.description}</div>}
              </td>
              {TOOLS.map((t) => {
                const st = s.links.find((l) => l.tool === t)?.state ?? "off";
                return (
                  <td key={t} className="center">
                    {st === "conflict" ? (
                      <span className="tone warn" title={`O ${TOOL_LABEL[t]} já tem uma skill própria com esse nome`}>conflito</span>
                    ) : (
                      <input type="checkbox" className="check" checked={st === "on"}
                        onChange={(e) => toggle(s.name, t, e.target.checked)}
                        title={`${st === "on" ? "Desligar" : "Ligar"} no ${TOOL_LABEL[t]}`} />
                    )}
                  </td>
                );
              })}
              <td className="right">
                <button className="small ghost" onClick={() => skillsApi.openFolder(s.name).catch(fail)}>Pasta</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {local.length > 0 && (
        <>
          <h3>Skills de uma ferramenta só</h3>
          <p className="sub">Estão só numa ferramenta. "Tornar global" move para a pasta global e deixa um link no lugar.</p>
          <table className="grid">
            <tbody>
              {local.map((s) => (
                <tr key={s.tool + s.name}>
                  <td>
                    <div className="skill-name">{s.name}</div>
                    {s.description && <div className="skill-desc">{s.description}</div>}
                  </td>
                  <td className="nowrap">
                    <span className={"mark mark-" + s.tool}>{s.tool === "claude" ? "C" : s.tool === "codex" ? "X" : "A"}</span>{" "}
                    {TOOL_LABEL[s.tool]}
                  </td>
                  <td className="right">
                    <button className="small" onClick={() => makeGlobal(s.tool, s.name)}>Tornar global</button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      <p className="hint">Pasta das skills globais: <code className="selectable">{state?.root}</code></p>
    </div>
  );
}
