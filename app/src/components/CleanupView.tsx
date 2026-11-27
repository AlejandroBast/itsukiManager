import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type CleanProgress, type CleanResult, type JunkItem, type JunkView, type Safety } from "../api";
import { formatCount, formatSize } from "../format";
import { useJob } from "../hooks";
import mascot from "../assets/images/mascot-cleanup.png";
import { Icon } from "./Icon";
import { Alert, Badge, BusyState, Checkbox, EmptyState, Modal, PageHeader } from "./ui";

const SAFETY: Record<Safety, { label: string; hint: string }> = {
  safe: { label: "Seguro", hint: "Se regenera solo" },
  regenerable: { label: "Regenerable", hint: "Se vuelve a crear o descargar (cuesta tiempo)" },
  review: { label: "Revisar", hint: "Puede contener cosas tuyas" },
  advice: { label: "Sugerencia", hint: "No se borra desde aquí" },
};

/** Lo que se marca solo al analizar: lo seguro que se puede limpiar ahora. */
function defaultSelection(view: JunkView): Set<number> {
  return new Set(
    view.categories
      .flatMap((c) => c.items)
      .filter((i) => i.safety === "safe" && !i.openApp && !(i.needsAdmin && !view.elevated))
      .map((i) => i.id),
  );
}

/** «Limpieza»: temporales, cachés y restos que se pueden borrar con seguridad. */
export function CleanupView(props: { defaultToRecycle: boolean; defaultIncludeOpen: boolean; onChanged: () => void }) {
  const { defaultToRecycle, defaultIncludeOpen, onChanged } = props;
  const [data, setData] = useState<JunkView | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [toRecycle, setToRecycle] = useState(defaultToRecycle);
  const [includeOpen, setIncludeOpen] = useState(defaultIncludeOpen);
  useEffect(() => setToRecycle(defaultToRecycle), [defaultToRecycle]);
  useEffect(() => setIncludeOpen(defaultIncludeOpen), [defaultIncludeOpen]);
  const [confirm, setConfirm] = useState(false);
  const [cleaning, setCleaning] = useState<CleanProgress | null>(null);
  const [result, setResult] = useState<CleanResult | null>(null);
  const { busy, progress, error, setError, run } = useJob();

  const load = useCallback(
    async (refresh: boolean) => {
      const view = await run(() => api.junkScan(refresh));
      if (view) {
        setData(view);
        setResult(null);
        setSelected(defaultSelection(view));
      }
    },
    [run],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  const items = useMemo(() => new Map((data?.categories ?? []).flatMap((c) => c.items).map((i) => [i.id, i])), [data]);
  const canSelect = (i: JunkItem) =>
    i.safety !== "advice" && !(i.needsAdmin && !data?.elevated) && (includeOpen || !i.openApp);
  const chosen = [...selected].map((id) => items.get(id)).filter((i): i is JunkItem => Boolean(i));
  const chosenBytes = chosen.reduce((acc, i) => acc + i.bytes, 0);

  const setMany = (list: JunkItem[], on: boolean) =>
    setSelected((prev) => {
      const next = new Set(prev);
      for (const i of list) {
        if (on) next.add(i.id);
        else next.delete(i.id);
      }
      return next;
    });

  const toggleExpanded = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const clean = async () => {
    setConfirm(false);
    const ids = chosen.map((i) => i.id);
    setCleaning({ done: 0, total: ids.length, bytes: 0 });
    const stop = await api.onCleanProgress(setCleaning).catch(() => null);
    try {
      const r = await api.junkClean(ids, toRecycle, includeOpen);
      setResult(r);
      setData(null);
      setSelected(new Set());
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      stop?.();
      setCleaning(null);
    }
  };

  const categories = data?.categories.filter((c) => c.safety !== "advice") ?? [];
  const advice = data?.categories.find((c) => c.safety === "advice");
  const byCategory = categories
    .map((c) => ({ label: c.label, bytes: c.items.filter((i) => selected.has(i.id)).reduce((acc, i) => acc + i.bytes, 0) }))
    .filter((c) => c.bytes > 0);

  return (
    <section className="page rise" aria-label="Limpieza">
      <PageHeader title="Limpieza" subtitle="Temporales, cachés y restos que se pueden borrar. Elige qué limpiar y pulsa el botón.">
        <button
          type="button"
          className="btn btn-outline btn-sm"
          disabled={busy || Boolean(cleaning)}
          onClick={() => void load(true)}
        >
          <Icon name="refresh" /> Volver a analizar
        </button>
      </PageHeader>

      <div className="card page-card">
        {error && (
          <Alert tone="error" onClose={() => setError(null)}>
            {error}
          </Alert>
        )}

        {cleaning && (
          <BusyState
            title="Limpiando…"
            detail={`${formatCount(cleaning.done)} de ${formatCount(cleaning.total)} ubicaciones · ${formatSize(cleaning.bytes)} liberados`}
          />
        )}
        {!cleaning && busy && (
          <BusyState
            title="Buscando basura en la unidad…"
            detail={progress ? `${formatCount(progress.files)} archivos revisados` : "Preparando el análisis"}
            onCancel={() => void api.cancelJob()}
          />
        )}

        {!cleaning && !busy && result && (
          <div className="state">
            <img className="hop" src={mascot} alt="" />
            <span className="state-icon ok" aria-hidden="true">
              <Icon name="check" />
            </span>
            <h2>
              {result.toRecycleBin ? "Enviado a la Papelera" : "Liberado"}: {formatSize(result.bytes)}
            </h2>
            <p>
              {formatCount(result.files)} archivos
              {result.skipped > 0 && ` · ${formatCount(result.skipped)} elementos omitidos (en uso o sin permiso)`}
            </p>
            {result.skippedOpen.length > 0 && (
              <p>
                Se saltaron las cachés de apps abiertas:{" "}
                {result.skippedOpen.map((s) => `${s.app} (${formatSize(s.bytes)})`).join(", ")}. Ciérralas y vuelve a limpiar para
                incluirlas.
              </p>
            )}
            <button type="button" className="btn btn-primary btn-md" onClick={() => void load(false)}>
              <Icon name="refresh" /> Volver a analizar
            </button>
          </div>
        )}

        {!cleaning && !busy && !result && data && categories.length === 0 && (
          <EmptyState title="Todo limpio" text="No se encontró basura conocida en la unidad del sistema." />
        )}

        {!cleaning && !busy && !result && data && categories.length > 0 && (
          <>
            <div className="stat-row">
              {(["safe", "regenerable", "review"] as const).map((s) => (
                <div key={s} className={`stat ${s}`}>
                  <span className="stat-label">{SAFETY[s].label}</span>
                  <strong>{formatSize(data[s])}</strong>
                  <small>{SAFETY[s].hint}</small>
                </div>
              ))}
            </div>
            {!data.elevated && categories.some((c) => c.items.some((i) => i.needsAdmin)) && (
              <Alert tone="warn">
                Algunas ubicaciones necesitan permisos de administrador (por ejemplo, las descargas de Windows Update). Para limpiarlas,
                abre ItsukiManager como administrador.
              </Alert>
            )}
            <div className="page-scroll">
              {categories.map((c, index) => {
                const available = c.items.filter(canSelect);
                const picked = available.filter((i) => selected.has(i.id)).length;
                const isOpen = expanded.has(c.id);
                const inUse = includeOpen ? 0 : c.items.filter((i) => i.openApp).length;
                return (
                  <div key={c.id} className="junk-cat stagger" style={{ "--i": index } as CSSProperties}>
                    <div className="row cols-junk">
                      <Checkbox
                        label={`Seleccionar ${c.label}`}
                        checked={available.length > 0 && picked === available.length}
                        indeterminate={picked > 0 && picked < available.length}
                        disabled={available.length === 0}
                        onChange={(on) => setMany(available, on)}
                      />
                      <button type="button" className="junk-name" aria-expanded={isOpen} onClick={() => toggleExpanded(c.id)}>
                        <Icon name="chevron" className={isOpen ? "open" : undefined} />
                        <span className="name">{c.label}</span>
                      </button>
                      <span className="tags">
                        {inUse > 0 && (
                          <Badge tone="warn" title="Cachés de aplicaciones abiertas: se saltan salvo que marques «Incluir apps abiertas»">
                            {formatCount(inUse)} en uso
                          </Badge>
                        )}
                        <Badge tone={c.safety} title={SAFETY[c.safety].hint}>
                          {SAFETY[c.safety].label}
                        </Badge>
                      </span>
                      <span className="num muted">
                        {formatCount(c.items.length)} {c.items.length === 1 ? "ubicación" : "ubicaciones"}
                      </span>
                      <span className="num strong">{formatSize(c.bytes)}</span>
                    </div>
                    {isOpen &&
                      c.items.map((i) => (
                        <div key={i.id} className="row cols-junk-item">
                          <Checkbox
                            label={`Seleccionar ${i.label}`}
                            checked={selected.has(i.id)}
                            disabled={!canSelect(i)}
                            onChange={(on) => setMany([i], on)}
                          />
                          <div className="cell-file">
                            <span className="name">{i.label}</span>
                            <span className="sub" title={i.path}>
                              {i.path}
                              {i.more > 0 && ` y ${formatCount(i.more)} más`}
                            </span>
                          </div>
                          <span className="tags">
                            {i.openApp && (
                              <Badge tone="warn" title="Está abierta: su caché se salta salvo que la incluyas">
                                Abierta: {i.openApp}
                              </Badge>
                            )}
                            {i.needsAdmin && !data.elevated && <Badge tone="advice">Requiere administrador</Badge>}
                          </span>
                          <span className="num">{formatSize(i.bytes)}</span>
                        </div>
                      ))}
                  </div>
                );
              })}
              {advice && advice.items.length > 0 && (
                <div className="advice">
                  <h3>Sugerencias (no se borran desde aquí)</h3>
                  {advice.items.map((i) => (
                    <div key={i.id} className="advice-item">
                      <div className="advice-title">
                        <strong>{i.label}</strong>
                        <span className="num">{formatSize(i.bytes)}</span>
                      </div>
                      {i.advice && <p>{i.advice}</p>}
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="page-foot">
              <Checkbox label="Enviar a la Papelera en vez de borrar" checked={toRecycle} onChange={setToRecycle}>
                Enviar a la Papelera
              </Checkbox>
              <Checkbox
                label="Incluir las cachés de aplicaciones abiertas"
                checked={includeOpen}
                onChange={(on) => {
                  setIncludeOpen(on);
                  if (!on) setMany([...items.values()].filter((i) => i.openApp), false);
                }}
              >
                Incluir apps abiertas
              </Checkbox>
              <span className="foot-total">
                Seleccionado: <strong>{formatSize(chosenBytes)}</strong>
              </span>
              <button
                type="button"
                className="btn btn-primary btn-md"
                disabled={chosen.length === 0}
                onClick={() => setConfirm(true)}
              >
                <Icon name="broom" /> Limpiar ahora
              </button>
            </div>
          </>
        )}

        {!cleaning && !busy && !result && !data && !error && (
          <EmptyState title="Busca basura" text="Analiza la unidad del sistema para ver cuánto espacio puedes liberar.">
            <button type="button" className="btn btn-primary btn-md" onClick={() => void load(false)}>
              <Icon name="play" /> Analizar
            </button>
          </EmptyState>
        )}
      </div>

      {confirm && (
        <Modal
          title={`¿Limpiar ${formatSize(chosenBytes)}?`}
          onClose={() => setConfirm(false)}
          actions={
            <>
              <button type="button" className="btn btn-outline btn-sm" autoFocus onClick={() => setConfirm(false)}>
                Cancelar
              </button>
              <button type="button" className="btn btn-danger btn-sm" onClick={() => void clean()}>
                <Icon name="broom" /> {toRecycle ? "Enviar a la Papelera" : "Borrar ahora"}
              </button>
            </>
          }
        >
          <p>Se van a limpiar {formatCount(chosen.length)} ubicaciones:</p>
          <ul className="modal-list">
            {byCategory.map((c) => (
              <li key={c.label}>
                <span>{c.label}</span>
                <strong>{formatSize(c.bytes)}</strong>
              </li>
            ))}
          </ul>
          <p className="modal-warn">
            {toRecycle
              ? "Irá a la Papelera: podrás restaurarlo y el espacio se libera al vaciarla."
              : "Se borrará de forma permanente (no pasa por la Papelera). Lo que esté en uso se salta."}
          </p>
        </Modal>
      )}
    </section>
  );
}
