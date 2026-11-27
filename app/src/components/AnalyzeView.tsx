import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, type DiskInfo, type FolderItem, type FoldersView } from "../api";
import { driveLabel, formatCount, formatDuration, formatPercent, formatSize } from "../format";
import { reveal, useJob } from "../hooks";
import { Icon } from "./Icon";
import { Alert, BusyState, EmptyState, IconButton, Modal, PageHeader, Segmented, SelectBox, type Option } from "./ui";

const GB = 1024 ** 3;
const MINS: Option<string>[] = [
  { value: String(GB / 2), label: "500 MB o más" },
  { value: String(GB), label: "1 GB o más" },
  { value: String(5 * GB), label: "5 GB o más" },
  { value: String(10 * GB), label: "10 GB o más" },
];

/** Quita una carpeta (y lo que contiene) y descuenta su tamaño de las carpetas que la contienen. */
function withoutFolder(view: FoldersView, gone: FolderItem): FoldersView {
  const inside = (f: FolderItem) => f.path === gone.path || f.path.startsWith(`${gone.path}\\`);
  const byId = new Map(view.folders.map((f) => [f.id, f]));
  const ancestors = new Set<number>();
  for (let p = gone.parent; p !== 0; p = byId.get(p)?.parent ?? 0) ancestors.add(p);
  return {
    ...view,
    folders: view.folders
      .filter((f) => !inside(f))
      .map((f) => (ancestors.has(f.id) ? { ...f, bytes: f.bytes - gone.bytes, files: f.files - gone.files } : f)),
  };
}

interface Props {
  drives: DiskInfo[];
  systemRoot: string;
  onChanged: () => void;
}

/** «Analizar disco»: las carpetas que más ocupan, en forma de árbol. */
export function AnalyzeView({ drives, systemRoot, onChanged }: Props) {
  const [root, setRoot] = useState(systemRoot);
  const [min, setMin] = useState(String(GB));
  const [data, setData] = useState<FoldersView | null>(null);
  const [open, setOpen] = useState<Set<number>>(new Set());
  const [confirm, setConfirm] = useState<FolderItem | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const { busy, progress, error, setError, run } = useJob();

  const load = useCallback(
    async (r: string, m: string, refresh: boolean) => {
      setNotice(null);
      const view = await run(() => api.largeFolders(r, Number(m), refresh));
      if (view) {
        setData(view);
        setOpen(new Set());
      }
      return view !== null;
    },
    [run],
  );

  useEffect(() => {
    void load(systemRoot, String(GB), false);
  }, [load, systemRoot]);

  const children = useMemo(() => {
    const map = new Map<number, FolderItem[]>();
    for (const f of data?.folders ?? []) {
      const list = map.get(f.parent);
      if (list) list.push(f);
      else map.set(f.parent, [f]);
    }
    return map;
  }, [data]);

  // `order`: posición entre sus hermanas (la cascada empieza de nuevo al desplegar una carpeta).
  const rows: { f: FolderItem; depth: number; order: number }[] = [];
  const walk = (parent: number, depth: number) => {
    for (const [order, f] of (children.get(parent) ?? []).entries()) {
      rows.push({ f, depth, order });
      if (open.has(f.id)) walk(f.id, depth + 1);
    }
  };
  walk(0, 0);

  const toggle = (id: number) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const recycle = async (f: FolderItem) => {
    setConfirm(null);
    try {
      await api.recyclePaths([f.path]);
      setData((d) => d && withoutFolder(d, f));
      setNotice(`«${f.name}» se envió a la Papelera. El espacio se libera al vaciarla.`);
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  const driveOptions: Option<string>[] = (drives.length ? drives : [{ root: systemRoot } as DiskInfo]).map((d) => ({
    value: d.root,
    label: driveLabel(d.root),
    hint: d.name ? `${d.name} · ${formatSize(d.used)} usados de ${formatSize(d.total)}` : undefined,
  }));
  const used = data?.scan.used || 1;
  const minLabel = MINS.find((o) => o.value === min)?.label.replace(" o más", "") ?? "";

  return (
    <section className="page rise" aria-label="Analizar disco">
      <PageHeader title="Analizar disco" subtitle="Las carpetas que más ocupan; despliega una para ver dentro.">
        <Segmented
          label="Unidad"
          options={driveOptions}
          value={root}
          disabled={busy}
          onChange={(r) => {
            const previous = root;
            setRoot(r);
            // Si se cancela o falla, se vuelve a la unidad de los resultados que se ven.
            void load(r, min, false).then((ok) => ok || setRoot(previous));
          }}
        />
        <SelectBox
          label="Tamaño mínimo"
          options={MINS}
          value={min}
          disabled={busy}
          onChange={(m) => {
            setMin(m);
            void load(root, m, false);
          }}
        />
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void load(root, min, true)}>
          <Icon name="refresh" /> Volver a analizar
        </button>
      </PageHeader>

      <div className="card page-card">
        {data && busy && (
          <div className="busy-strip" role="status">
            <span className="spinner" aria-hidden="true" />
            Analizando {driveLabel(root)}… {progress ? `${formatCount(progress.files)} archivos` : ""}
            <button type="button" className="link" onClick={() => void api.cancelJob()}>
              Cancelar
            </button>
          </div>
        )}
        {data && !busy && (
          <div className="page-meta">
            <span>
              <strong>{formatCount(data.folders.length)}</strong> carpetas de {minLabel} o más en {driveLabel(data.scan.root)}
            </span>
            <span>
              <strong>{formatSize(data.scan.used)}</strong>
              {data.scan.total ? ` usados de ${formatSize(data.scan.total)}` : " escaneados"}
            </span>
            <span>
              {formatCount(data.scan.files)} archivos en {formatDuration(data.scan.elapsedMs)}
            </span>
            {data.scan.denied > 0 && (
              <span title="Abre ItsukiManager como administrador para leerlas todas">
                {formatCount(data.scan.denied)} carpetas sin acceso
              </span>
            )}
          </div>
        )}
        {error && (
          <Alert tone="error" onClose={() => setError(null)}>
            {error}
          </Alert>
        )}
        {notice && (
          <Alert tone="ok" onClose={() => setNotice(null)}>
            {notice}
          </Alert>
        )}

        {!data && busy && (
          <BusyState
            title={`Analizando ${driveLabel(root)}…`}
            detail={progress ? `${formatCount(progress.files)} archivos · ${formatSize(progress.bytes)}` : "Preparando el escaneo"}
            onCancel={() => void api.cancelJob()}
          />
        )}
        {!data && !busy && (
          <EmptyState title="Analiza una unidad" text="Elige la unidad y pulsa el botón para ver qué carpetas ocupan más espacio.">
            <button type="button" className="btn btn-primary btn-md" onClick={() => void load(root, min, false)}>
              <Icon name="play" /> Analizar {driveLabel(root)}
            </button>
          </EmptyState>
        )}
        {data && rows.length === 0 && !busy && (
          <EmptyState title="Nada tan grande" text={`No hay carpetas de ${minLabel} o más en ${driveLabel(data.scan.root)}.`} />
        )}
        {data && rows.length > 0 && (
          <div className="page-scroll" role="tree" aria-label="Carpetas grandes">
            <div className="list-head cols-folders" aria-hidden="true">
              <span>Carpeta</span>
              <span>Ocupa</span>
              <span className="num">Tamaño</span>
              <span className="num">Disco</span>
              <span />
            </div>
            {rows.map(({ f, depth, order }) => {
              const kids = children.has(f.id);
              const isOpen = open.has(f.id);
              return (
                <div
                  key={f.id}
                  className="row cols-folders stagger"
                  style={{ "--i": order } as CSSProperties}
                  role="treeitem" aria-expanded={kids ? isOpen : undefined}>
                  <div className="cell-name" style={{ paddingLeft: `${depth * 1.6}rem` }}>
                    {kids ? (
                      <button
                        type="button"
                        className="expander"
                        aria-label={isOpen ? `Contraer ${f.name}` : `Desplegar ${f.name}`}
                        onClick={() => toggle(f.id)}
                      >
                        <Icon name="chevron" className={isOpen ? "open" : undefined} />
                      </button>
                    ) : (
                      <span className="expander" />
                    )}
                    <Icon name="folder" className="row-icon" />
                    <span className="name" title={f.path}>
                      {f.name}
                    </span>
                  </div>
                  <span className="meter" aria-hidden="true">
                    <span style={{ width: `${Math.max(1, (f.bytes / used) * 100)}%` }} />
                  </span>
                  <span className="num strong">{formatSize(f.bytes)}</span>
                  <span className="num muted">{formatPercent(f.bytes, used)}</span>
                  <span className="row-actions">
                    <IconButton icon="open" label="Mostrar en el Explorador" onClick={() => reveal(f.path, setError)} />
                    <IconButton icon="trash" label="Enviar a la Papelera" danger onClick={() => setConfirm(f)} />
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {confirm && (
        <Modal
          title="¿Enviar a la Papelera?"
          onClose={() => setConfirm(null)}
          actions={
            <>
              <button type="button" className="btn btn-outline btn-sm" autoFocus onClick={() => setConfirm(null)}>
                Cancelar
              </button>
              <button type="button" className="btn btn-danger btn-sm" onClick={() => void recycle(confirm)}>
                <Icon name="trash" /> Enviar a la Papelera
              </button>
            </>
          }
        >
          <p>
            La carpeta <strong>«{confirm.name}»</strong> ({formatSize(confirm.bytes)}, {formatCount(confirm.files)} archivos) irá a la
            Papelera. Podrás restaurarla desde allí; el espacio se libera al vaciarla.
          </p>
          <p className="modal-path">{confirm.path}</p>
          <p className="modal-warn">Las carpetas de Windows, de programas y las carpetas base del usuario están protegidas.</p>
        </Modal>
      )}
    </section>
  );
}
