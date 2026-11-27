import face from "../assets/images/mascot-banner.png";
import { Icon } from "./Icon";

/** Estrellas decorativas del banner: posición (en rem desde la derecha y arriba), tamaño y opacidad. */
const STARS = [
  { right: 35.85, top: 1.415, size: 1.8, opacity: 0.13 },
  { right: 4.81, top: 1.5, size: 1.5, opacity: 0.16 },
  { right: 2.325, top: 1.3225, size: 1.1, opacity: 0.16 },
  { right: 2.7275, top: 5.975, size: 2.3, opacity: 0.35 },
];

export function WelcomeBanner() {
  return (
    <section className="welcome rise" aria-label="Bienvenida">
      <h1>¡Hola! 👋</h1>
      <p>Analiza y gestiona el almacenamiento de tu PC con ItsukiManager.</p>
      {STARS.map((s, i) => (
        <span
          key={i}
          className="deco-star"
          style={{ right: `${s.right}rem`, top: `${s.top}rem`, width: `${s.size}rem`, height: `${s.size}rem`, opacity: s.opacity }}
        >
          <Icon name="star" />
        </span>
      ))}
      <img className="face" src={face} alt="" />
    </section>
  );
}
