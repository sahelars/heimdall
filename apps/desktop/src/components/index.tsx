/**
 * The shared pieces of the visual system (SPEC §15).
 *
 * Every border here is 1px solid and squared, and nothing carries color: state
 * is communicated through weight, rules, and wording instead.
 */

import type { ReactNode } from "react";

import type { DomainError } from "../api/types";

export function Panel({
  title,
  description,
  action,
  children,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="panel">
      <div className="panel__title">
        <h2>{title}</h2>
        {action}
      </div>
      {description ? <p className="panel__description">{description}</p> : null}
      {children}
    </section>
  );
}

export function Button({
  children,
  onClick,
  disabled,
  primary,
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  primary?: boolean;
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      className={primary ? "button button--primary" : "button"}
      onClick={onClick}
      disabled={disabled}
    >
      {children}
    </button>
  );
}

export function Field({
  label,
  value,
  onChange,
  placeholder,
  hint,
  grow,
  id,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  hint?: string;
  grow?: boolean;
  id: string;
}) {
  return (
    <div className={grow ? "field field--grow" : "field"}>
      <label className="field__label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        className="input"
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      {hint ? <span className="field__hint">{hint}</span> : null}
    </div>
  );
}

/**
 * A structured failure, shown so it stays actionable.
 *
 * The code is what a user can search for or quote, the message is what to do
 * about it, and the details are the specifics — none of which should ever be
 * flattened into "something went wrong".
 */
export function Failure({ error, stderr }: { error: DomainError; stderr?: string }) {
  const details = error.details && Object.keys(error.details).length > 0 ? error.details : null;
  return (
    <div className="notice notice--failure" role="alert">
      <span className="notice__code">{error.code}</span>
      <p>{error.message}</p>
      {details ? <pre className="notice__detail">{JSON.stringify(details, null, 2)}</pre> : null}
      {stderr ? <pre className="notice__detail">{stderr}</pre> : null}
    </div>
  );
}

export function Notice({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="notice" role="status">
      <span className="notice__code">{title}</span>
      {children}
    </div>
  );
}

export function Facts({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="facts">
      {rows.map(([label, value]) => (
        <div key={label} style={{ display: "contents" }}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
