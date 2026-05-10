import { Link } from "react-router-dom";
import { serviceCards } from "../data/services";

export default function Services() {
  return (
    <section id="services" className="border-t border-border py-20 md:py-28">
      <div className="mx-auto max-w-6xl px-6">
        <h2 className="text-center text-3xl font-bold text-text-primary md:text-4xl">
          Services that ship today
        </h2>
        <p className="mx-auto mt-4 max-w-2xl text-center text-text-secondary">
          Each service is a Rust module, a TOML schema, and a CLI surface — all
          glued together so an agent can address it the same way regardless of
          provider quirks.
        </p>

        <div className="mt-14 grid gap-6 md:grid-cols-2 lg:grid-cols-3">
          {serviceCards.map((s) => (
            <div
              key={s.name}
              className="group flex flex-col rounded-xl border border-border bg-surface-alt p-6 transition-all hover:border-accent/40 hover:bg-surface-hover"
            >
              <div className="flex items-center justify-between">
                <h3 className={`text-xl font-semibold ${s.colorVar}`}>
                  {s.displayName}
                </h3>
                <code className="rounded-md border border-border bg-surface px-2 py-0.5 text-xs text-text-dim">
                  zad {s.name}
                </code>
              </div>
              <p className="mt-3 text-sm leading-relaxed text-text-secondary">{s.tagline}</p>

              <div className="mt-4 flex flex-wrap gap-1.5">
                {s.verbs.map((v) => (
                  <span
                    key={v}
                    className="rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-[11px] text-text-dim"
                  >
                    {v}
                  </span>
                ))}
              </div>

              <div className="mt-auto pt-5 text-xs text-text-dim">
                <div>{s.auth}</div>
                <Link
                  to={`/manual/${s.manpage}`}
                  className="mt-2 inline-block text-accent hover:text-accent-light transition-colors"
                >
                  Manpage →
                </Link>
              </div>
            </div>
          ))}
        </div>

        <p className="mx-auto mt-10 max-w-2xl text-center text-sm text-text-dim">
          Want one that isn't here? Adding a new service is a checklist: a
          module under <code className="rounded bg-surface-alt px-1.5 py-0.5">crates/zad/src/service/&lt;name&gt;/</code>,
          a row in the registry, a permissions schema, a manpage, and an example.
          See the{" "}
          <Link to="/docs/services" className="text-accent hover:text-accent-light underline">
            Services guide
          </Link>
          .
        </p>
      </div>
    </section>
  );
}
