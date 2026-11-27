import mascot from "../assets/images/mascot-cleanup.png";
import type { Section } from "./Sidebar";

/** Texto de cada sección que todavía no tiene pantalla propia. */
const INFO: Record<Exclude<Section, "inicio">, { title: string; text: string; cli?: string }> = {
  analizar: {
    title: "Analizar disco",
    text: "Aquí podrás recorrer tu disco carpeta por carpeta. Mientras tanto, puedes verlo desde la terminal.",
    cli: "itsuki carpetas",
  },
  grandes: {
    title: "Archivos grandes",
    text: "Aquí verás los archivos que más ocupan. Mientras tanto, puedes verlos desde la terminal.",
    cli: "itsuki archivos --top 50",
  },
  duplicados: {
    title: "Duplicados",
    text: "Aquí encontrarás los archivos repetidos. Mientras tanto, puedes buscarlos desde la terminal.",
    cli: "itsuki duplicados",
  },
  limpieza: {
    title: "Limpieza",
    text: "Aquí podrás limpiar temporales y cachés. Mientras tanto, puedes analizarlos desde la terminal.",
    cli: "itsuki limpieza",
  },
  configuracion: {
    title: "Configuración",
    text: "Las opciones de ItsukiManager estarán aquí.",
  },
  acerca: {
    title: "ItsukiManager 0.1.0",
    text: "Gestor de almacenamiento para Windows. La mascota está inspirada en Itsuki Nakano, personaje de terceros.",
  },
};

export function SectionPlaceholder({ section }: { section: Exclude<Section, "inicio"> }) {
  const info = INFO[section];
  return (
    <section className="section rise">
      <img src={mascot} alt="" />
      <h2>{info.title}</h2>
      <p>{info.text}</p>
      {info.cli && <code>{info.cli}</code>}
    </section>
  );
}
