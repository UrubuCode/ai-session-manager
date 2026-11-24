import { useCallback, useEffect, useState } from "react";
import { exeApi, type ExeInfo, type ExeSource, type ToolId } from "./api";

const SOURCE: Record<ExeSource, string> = {
  custom: "registrado",
  path: "PATH",
  known: "local conhecido",
};

export function ExecutablesView(props: {
  setStatus: (s: { text: string; error?: boolean }) => void;
  onChanged: () => void;
}) {
  const [list, setList] = useState<ExeInfo[] | null>(null);
  const fail = (e: unknown) => props.setStatus({ text: String(e), error: true });

  const load = useCallback(() => exeApi.get().then(setList).catch(fail), []);
  useEffect(() => { load(); }, [load]);

  const save = async (tool: ToolId, label: string, path: string | null) => {
    try {
      setList(await exeApi.set(tool, path));
      props.onChanged();
      props.setStatus({ text: path ? `${label}: caminho registrado` : `${label}: voltou para a busca automática` });
    } catch (e) { fail(e); }
  };

  return (
    <div className="page wide">
      <div className="page-head">
        <div>
          <h2>Executáveis</h2>
          <p className="sub">
            Onde o app encontra cada ferramenta. A busca olha o PATH, o instalador nativo, npm/pnpm/bun/scoop e
            extensões do VS Code, Cursor e Windsurf. Se ela não achar, ou achar o errado, registre o caminho aqui.
          </p>
        </div>
        <div className="detail-actions">
          <button className="ghost" onClick={() => { load(); props.setStatus({ text: "Busca refeita" }); }}>Procurar de novo</button>
        </div>
      </div>

      <div className="cards">
        {list?.map((x) => <ExeCard key={x.tool} x={x} onSave={(p) => save(x.tool, x.label, p)} />)}
      </div>
    </div>
  );
}

function ExeCard({ x, onSave }: { x: ExeInfo; onSave: (path: string | null) => void }) {
  const [draft, setDraft] = useState(x.custom ?? "");
  useEffect(() => { setDraft(x.custom ?? ""); }, [x.custom]);

  return (
    <div className="card wide">
      <div className="card-title">
        <span className={"mark mark-" + x.tool}>{x.tool === "claude" ? "C" : x.tool === "codex" ? "X" : "A"}</span>{" "}
        {x.label} <span className="muted-inline">({x.command})</span>
      </div>
      <div className="card-body">
        {x.resolved ? (
          <div className="args-row">
            <code className="selectable">{x.resolved.path}</code>
            <span className="badge">{SOURCE[x.resolved.source]}</span>
          </div>
        ) : (
          <span className="tone bad">Não encontrado — registre o caminho abaixo</span>
        )}
        {x.customMissing && (
          <small className="tone bad">O caminho registrado não existe mais: {x.custom}</small>
        )}

        {x.candidates.length > 0 && (
          <div className="exe-candidates">
            <small>Encontrados:</small>
            {x.candidates.map((c) => (
              <div key={c.path} className="args-row">
                <code className="selectable">{c.path}</code>
                <span className="muted-inline">{SOURCE[c.source]}</span>
                <button className="small ghost" onClick={() => onSave(c.path)}
                  disabled={x.resolved?.source === "custom" && x.resolved.path === c.path}>Usar este</button>
              </div>
            ))}
          </div>
        )}

        <div className="args-row exe-input">
          <input className="search" style={{ flex: 1 }} placeholder={`Caminho completo do ${x.command}.exe (ou da pasta)`}
            value={draft} onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && draft.trim()) onSave(draft); }} />
          <button className="small" onClick={() => onSave(draft)} disabled={!draft.trim() || draft === x.custom}>Registrar</button>
          {x.custom && <button className="small ghost" onClick={() => onSave(null)}>Automático</button>}
        </div>
      </div>
    </div>
  );
}
