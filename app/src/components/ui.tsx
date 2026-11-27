import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import mascot from "../assets/images/mascot-cleanup.png";
import { Icon, type IconName } from "./Icon";

/** Título de sección con su descripción y la barra de acciones. */
export function PageHeader({ title, subtitle, children }: { title: string; subtitle: string; children?: ReactNode }) {
  return (
    <header className="page-head">
      <div className="page-heading">
        <h1>{title}</h1>
        <p>{subtitle}</p>
      </div>
      {children && <div className="toolbar">{children}</div>}
    </header>
  );
}

/** Botón cuadrado con solo un icono (acciones de cada fila). */
export function IconButton(props: {
  icon: IconName;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      className={props.danger ? "icon-btn danger" : "icon-btn"}
      aria-label={props.label}
      title={props.label}
      disabled={props.disabled}
      onClick={props.onClick}
    >
      <Icon name={props.icon} />
    </button>
  );
}

export interface Option<T extends string> {
  value: T;
  label: string;
  hint?: string;
}

/** Selector de opciones en forma de píldoras (unidades, alcance). */
export function Segmented<T extends string>(props: {
  label: string;
  options: Option<T>[];
  value: T;
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={props.label}>
      {props.options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === props.value}
          className={o.value === props.value ? "seg active" : "seg"}
          title={o.hint}
          disabled={props.disabled}
          onClick={() => props.onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Lista desplegable con el estilo de la app (usa el control nativo para ser accesible). */
export function SelectBox<T extends string>(props: {
  label: string;
  options: Option<T>[];
  value: T;
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  return (
    <label className="select" title={props.label}>
      <span className="sr-only">{props.label}</span>
      <select value={props.value} disabled={props.disabled} onChange={(e) => props.onChange(e.target.value as T)}>
        {props.options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <Icon name="chevron" className="select-chevron" />
    </label>
  );
}

/** Casilla con estado intermedio (para seleccionar categorías enteras). */
export function Checkbox(props: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  indeterminate?: boolean;
  disabled?: boolean;
  children?: ReactNode;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = Boolean(props.indeterminate);
  }, [props.indeterminate]);
  return (
    <label className={props.disabled ? "check disabled" : "check"} title={props.children ? undefined : props.label}>
      <input
        ref={ref}
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        aria-label={props.children ? undefined : props.label}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      <span className="check-box" aria-hidden="true">
        <Icon name="check" />
      </span>
      {props.children && <span className="check-text">{props.children}</span>}
    </label>
  );
}

export type Tone = "safe" | "regenerable" | "review" | "advice" | "warn" | "info";

export function Badge({ tone, children, title }: { tone: Tone; children: ReactNode; title?: string }) {
  return (
    <span className={`badge ${tone}`} title={title}>
      {children}
    </span>
  );
}

/** Diálogo modal (confirmaciones). Se cierra con Esc o tocando fuera. */
export function Modal(props: { title: string; onClose: () => void; children: ReactNode; actions: ReactNode }) {
  const { onClose } = props;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return createPortal(
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <h2 id="modal-title">{props.title}</h2>
        <div className="modal-body">{props.children}</div>
        <div className="modal-actions">{props.actions}</div>
      </div>
    </div>,
    document.body,
  );
}

/** Estado vacío: mascota, texto y la acción principal. */
export function EmptyState({ title, text, children }: { title: string; text: string; children?: ReactNode }) {
  return (
    <div className="state">
      <img src={mascot} alt="" />
      <h2>{title}</h2>
      <p>{text}</p>
      {children}
    </div>
  );
}

/** Trabajo en curso con su progreso y, si se puede, el botón de cancelar. */
export function BusyState({ title, detail, onCancel }: { title: string; detail?: string; onCancel?: () => void }) {
  return (
    <div className="state" role="status" aria-live="polite">
      <span className="spinner big" aria-hidden="true" />
      <h2>{title}</h2>
      {detail && <p>{detail}</p>}
      {onCancel && (
        <button type="button" className="btn btn-outline btn-sm" onClick={onCancel}>
          Cancelar
        </button>
      )}
    </div>
  );
}

/** Aviso dentro de una tarjeta (errores o advertencias). */
export function Alert({ tone, children, onClose }: { tone: "error" | "warn" | "ok"; children: ReactNode; onClose?: () => void }) {
  return (
    <div className={`alert ${tone}`} role={tone === "error" ? "alert" : "status"}>
      <Icon name={tone === "ok" ? "check" : "warning"} />
      <div className="alert-text">{children}</div>
      {onClose && <IconButton icon="close" label="Cerrar aviso" onClick={onClose} />}
    </div>
  );
}
