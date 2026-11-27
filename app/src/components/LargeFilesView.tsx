import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";
import { api, type DiskInfo, type FileItem, type FilesView } from "../api";
import { driveLabel, formatCount, formatDate, formatPercent, formatSize } from "../format";
import { reveal, useJob } from "../hooks";
import { Icon } from "./Icon";
import { Alert, BusyState, EmptyState, IconButton, Modal, PageHeader, Segmented, SelectBox, type Option } from "./ui";

const TOPS: Option<string>[] = [
  { value: "50", label: "Los 50 más grandes" },
  { value: "100", label: "Los 100 más grandes" },
  { value: "200", label: "Los 200 más grandes" },
];

interface Props {
  drives: DiskInfo[];
  systemRoot: string;
  onChanged: () => void;
}

/** «Archivos grandes»: los archivos que más ocupan en una unidad. */
export function LargeFilesView({ drives, systemRoot, onChanged }: Props) {
  const [root, setRoot] = useState(systemRoot);
  const [top, setTop] = useState("50");
  const [data, setData] = useState<FilesView | null>(null);
  const [confirm, setConfirm] = useState<FileItem | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const { busy, progress, error, setError, run } = useJob();

  const load = useCallback(
    async (r: string, t: string, refresh: boolean) => {
      setNotice(null);
      const view = await run(() => api.largeFiles(r, Number(t), refresh));
      if (view) setData(view);
      return view !== null;
    },
    [run],
  );

  useEffect(() => {
    void load(systemRoot, "50", false);
  }, [load, systemRoot]);

  const recycle = async (f: FileItem) => {
    setConfirm(null);
    try {
      await api.recyclePaths([f.path]);
      setData((d) => d && { ...d, files: d.files.filter((x) => x.path !== f.path) });
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
  const sum = data?.files.reduce((acc, f) => acc + f.bytes, 0) ?? 0;

  return (
    <section className="page rise" aria-label="Archivos grandes">
      <PageHeader title="Archivos grandes" subtitle="Lo que más ocupa. Envía a la Papelera lo que no necesites.">
        <Segmented
          label="Unidad"
          options={driveOptions}
          value={root}
          disabled={busy}
          onChange={(r) => {
            const previous = root;
            setRoot(r);
            // Si se cancela o falla, se vuelve a la unidad de los resultados que se ven.
            void load(r, top, false).then((ok) => ok || setRoot(previous));
          }}
        />
        <SelectBox
          label="Cuántos mostrar"
          options={TOPS}
          value={top}
          disabled={busy}
          onChange={(t) => {
            setTop(t);
            void load(root, t, false);
          }}
        />
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void load(root, top, true)}>
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
        {data && !busy && data.files.length > 0 && (
          <div className="page-meta">
            <span>
              Los <strong>{formatCount(data.files.length)}</strong> archivos más grandes de {driveLabel(data.scan.root)} suman{" "}
              <strong>{formatSize(sum)}</strong>
            </span>
            <span>{formatPercent(sum, data.scan.used || 1)} del espacio usado</span>
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
          <EmptyState title="Busca los archivos más grandes" text="Elige la unidad y pulsa el botón para listar los archivos que más ocupan.">
            <button type="button" className="btn btn-primary btn-md" onClick={() => void load(root, top, false)}>
              <Icon name="play" /> Buscar en {driveLabel(root)}
            </button>
          </EmptyState>
        )}
        {data && data.files.length === 0 && !busy && (
          <EmptyState title="Sin archivos" text={`No se encontraron archivos en ${driveLabel(data.scan.root)}.`} />
        )}
        {data && data.files.length > 0 && (
          <div className="page-scroll">
            <div className="list-head cols-files" aria-hidden="true">
              <span>Archivo</span>
              <span>Tipo</span>
              <span>Modificado</span>
              <span className="num">Tamaño</span>
              <span />
            </div>
            {data.files.map((f, i) => (
              <div key={f.path} className="row cols-files stagger" style={{ "--i": i } as CSSProperties}>
                <div className="cell-file">
                  <span className="name" title={f.name}>
                    {f.name}
                  </span>
                  <span className="sub" title={f.folder}>
                    {f.folder}
                  </span>
                </div>
                <span className="muted ellipsis" title={f.kind}>
                  {f.kind}
                </span>
                <span className="muted">{formatDate(f.modified)}</span>
                <span className="num strong">{formatSize(f.bytes)}</span>
                <span className="row-actions">
                  <IconButton icon="open" label="Mostrar en el Explorador" onClick={() => reveal(f.path, setError)} />
                  <IconButton icon="trash" label="Enviar a la Papelera" danger onClick={() => setConfirm(f)} />
                </span>
              </div>
            ))}
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
            <strong>«{confirm.name}»</strong> ({formatSize(confirm.bytes)}) irá a la Papelera. Podrás restaurarlo desde allí; el
            espacio se libera al vaciarla.
          </p>
          <p className="modal-path">{confirm.path}</p>
          {confirm.hidden && <p className="modal-warn">Es un archivo oculto o de sistema: asegúrate de que no lo necesita ningún programa.</p>}
        </Modal>
      )}
    </section>
  );
}
