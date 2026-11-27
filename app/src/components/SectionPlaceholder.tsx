import mascot from "../assets/images/mascot-cleanup.png";

/** Secciones informativas que todavía no tienen pantalla propia. */
const INFO = {
  configuracion: {
    title: "Configuración",
    text: "Las opciones de ItsukiManager estarán aquí.",
  },
  acerca: {
    title: "ItsukiManager 0.1.0",
    text: "Gestor de almacenamiento para Windows. La mascota está inspirada en Itsuki Nakano, personaje de terceros.",
  },
};

export function SectionPlaceholder({ section }: { section: keyof typeof INFO }) {
  const info = INFO[section];
  return (
    <section className="section rise">
      <img src={mascot} alt="" />
      <h2>{info.title}</h2>
      <p>{info.text}</p>
    </section>
  );
}
