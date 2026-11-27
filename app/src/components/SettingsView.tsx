import type { ReactNode } from "react";
import type { DiskInfo, EngineChoice, Settings } from "../api";
import { driveLabel } from "../format";
import { DUP_MINS } from "./DuplicatesView";
import { Icon, type IconName } from "./Icon";
import { Alert, PageHeader, Segmented, SelectBox, Switch, type Option } from "./ui";

export const DEFAULT_SETTINGS: Settings = {
  engine: "auto",
  dedupeHardlinks: true,
  cleanToRecycle: false,
  cleanIncludeOpen: false,
  tempMinAgeDays: 1,
  dupesScope: "personal",
  dupesMin: 1024 * 1024,
};

const ENGINES: (Option<EngineChoice> & { detail: string })[] = [
  { value: "auto", label: "Auto", detail: "El más rápido disponible: MFT como administrador en unidades NTFS; si no, el nativo." },
  { value: "mft", label: "MFT", detail: "Lee la tabla de archivos de NTFS directamente. Solo como administrador y en unidades completas." },
  { value: "native", label: "Nativo", detail: "API nativa de Windows. Funciona sin permisos de administrador." },
  { value: "portable", label: "Portable", detail: "El más lento y compatible. Útil para comparar resultados." },
];

const TEMP_AGES: Option<string>[] = [
  { value: "0", label: "Todos" },
  { value: "1", label: "1 día" },
  { value: "7", label: "7 días" },
  { value: "30", label: "30 días" },
];

function SettingsCard(props: { icon: IconName; tone: string; title: string; text: string; children: ReactNode }) {
  return (
    <section className="card settings-card" aria-label={props.title}>
      <header className="settings-head">
        <span className={`settings-icon ${props.tone}`}>
          <Icon name={props.icon} />
        </span>
        <div>
          <h2 className="card-title">{props.title}</h2>
          <p>{props.text}</p>
        </div>
      </header>
      <div className="settings-body">{props.children}</div>
    </section>
  );
}

/** Ajuste con su explicación; `inline` pone el control a la derecha (interruptores). */
function Setting(props: { title: string; hint: string; inline?: boolean; children: ReactNode }) {
  return (
    <div className={props.inline ? "setting inline" : "setting"}>
      <div className="setting-text">
        <strong>{props.title}</strong>
        <span>{props.hint}</span>
      </div>
      <div className="setting-control">{props.children}</div>
    </div>
  );
}

interface Props {
  settings: Settings;
  drives: DiskInfo[];
  error: string | null;
  onChange: (settings: Settings) => void;
  onDismissError: () => void;
}

/** «Configuración»: cómo se escanea, cómo se limpia y cómo se buscan duplicados. Se guarda al momento. */
export function SettingsView({ settings, drives, error, onChange, onDismissError }: Props) {
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => onChange({ ...settings, [key]: value });
  const engine = ENGINES.find((e) => e.value === settings.engine) ?? ENGINES[0];
  const scopes: Option<string>[] = [
    { value: "personal", label: "Carpetas personales" },
    ...drives.map((d) => ({ value: d.root, label: `${driveLabel(d.root)} ${d.name}` })),
  ];
  const isDefault = JSON.stringify(settings) === JSON.stringify(DEFAULT_SETTINGS);

  return (
    <section className="page rise" aria-label="Configuración">
      <PageHeader title="Configuración" subtitle="Los cambios se guardan al momento y se usan en el próximo análisis.">
        <button
          type="button"
          className="btn btn-outline btn-sm"
          disabled={isDefault}
          onClick={() => onChange(DEFAULT_SETTINGS)}
        >
          <Icon name="refresh" /> Restablecer valores
        </button>
      </PageHeader>

      {error && (
        <Alert tone="error" onClose={onDismissError}>
          {error}
        </Alert>
      )}

      <div className="settings-grid">
        <SettingsCard icon="search" tone="violet" title="Escaneo" text="Cómo se recorre el disco.">
          <Setting title="Motor de escaneo" hint={engine.detail}>
            <Segmented label="Motor de escaneo" options={ENGINES} value={settings.engine} onChange={(v) => set("engine", v)} />
          </Setting>
          <Setting title="Enlaces duros" hint="Cuenta una sola vez los archivos que aparecen en varias carpetas (como hace Windows)." inline>
            <Switch label="Contar los enlaces duros una sola vez" checked={settings.dedupeHardlinks} onChange={(v) => set("dedupeHardlinks", v)} />
          </Setting>
          <p className="settings-note">Al cambiar el escaneo se olvida lo ya analizado: la próxima vez se vuelve a leer el disco.</p>
        </SettingsCard>

        <SettingsCard icon="trash" tone="coral" title="Limpieza" text="Qué se marca al abrir Limpieza.">
          <Setting title="Enviar a la Papelera" hint="En vez de borrar. Podrás restaurarlo, pero el espacio no se libera hasta vaciarla." inline>
            <Switch label="Enviar a la Papelera en vez de borrar" checked={settings.cleanToRecycle} onChange={(v) => set("cleanToRecycle", v)} />
          </Setting>
          <Setting title="Incluir apps abiertas" hint="Limpia también las cachés de programas en uso. Puede cerrarles la sesión o hacer que fallen." inline>
            <Switch
              label="Incluir las cachés de aplicaciones abiertas"
              checked={settings.cleanIncludeOpen}
              onChange={(v) => set("cleanIncludeOpen", v)}
            />
          </Setting>
          <Setting title="Antigüedad de los temporales" hint="Solo se borran los temporales que llevan al menos este tiempo sin cambiar.">
            <Segmented
              label="Antigüedad mínima de los temporales"
              options={TEMP_AGES}
              value={String(settings.tempMinAgeDays)}
              onChange={(v) => set("tempMinAgeDays", Number(v))}
            />
          </Setting>
        </SettingsCard>

        <SettingsCard icon="files" tone="blue" title="Duplicados" text="Valores con los que empieza la búsqueda.">
          <Setting title="Dónde buscar" hint="Las carpetas personales son Escritorio, Documentos, Descargas, Imágenes, Vídeos, Música y OneDrive.">
            <SelectBox label="Dónde buscar" options={scopes} value={settings.dupesScope} onChange={(v) => set("dupesScope", v)} />
          </Setting>
          <Setting title="Tamaño mínimo" hint="Los archivos más pequeños no se comparan (es más rápido).">
            <SelectBox
              label="Tamaño mínimo"
              options={DUP_MINS}
              value={String(settings.dupesMin)}
              onChange={(v) => set("dupesMin", Number(v))}
            />
          </Setting>
        </SettingsCard>
      </div>
    </section>
  );
}
