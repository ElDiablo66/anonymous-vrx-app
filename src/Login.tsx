import { useState, type FormEvent } from "react";
import { backend, errorText, type Me } from "./backend";
import logo from "./assets/logo.svg";

const methodNames: Record<string, { label: string; hint: string }> = {
  totp: { label: "Authenticator app", hint: "The 6-digit code from your authenticator app." },
  emailOtp: { label: "Email code", hint: "VRChat sent a 6-digit code to your email address." },
  otp: { label: "Recovery code", hint: "One of the recovery codes you saved when you turned on two-step sign-in." },
};

export function Login({ onSignedIn }: { onSignedIn: (me: Me) => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [methods, setMethods] = useState<string[] | null>(null);
  const [method, setMethod] = useState("totp");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");

  async function signIn(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setProblem("");
    try {
      const reply = await backend.login(username, password);
      setPassword("");
      if (reply.status === "signedIn" && reply.me) {
        onSignedIn(reply.me);
      } else if (reply.status === "needsCode") {
        const offered = reply.methods.filter((m) => m in methodNames);
        setMethods(offered);
        setMethod(offered.find((m) => m !== "otp") ?? offered[0] ?? "totp");
      }
    } catch (err) {
      const kind = (err as { kind?: string }).kind;
      setProblem(kind === "unauthorized" ? "VRChat did not accept that username and password." : errorText(err));
    } finally {
      setBusy(false);
    }
  }

  async function sendCode(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setProblem("");
    try {
      const reply = await backend.verifyCode(method, code);
      if (reply.me) onSignedIn(reply.me);
    } catch (err) {
      const kind = (err as { kind?: string }).kind;
      setProblem(kind === "badCode" ? "That code was not accepted. Check it and try again." : errorText(err));
      setCode("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="login">
      <div className="login-light" aria-hidden="true" />
      <section className="login-card">
        <p className="wordmark">
          <img src={logo} alt="" />
          <span>
            Anonymous <b>VRX</b>
          </span>
        </p>
        <h1>See who is around</h1>
        {methods === null ? (
          <form onSubmit={signIn}>
            <p className="lede">
              Sign in with your VRChat account. Your password goes straight to VRChat and is not saved on this PC.
            </p>
            <label>
              Username or email
              <input autoFocus autoComplete="username" value={username} onChange={(e) => setUsername(e.target.value)} required />
            </label>
            <label>
              Password
              <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
            </label>
            {problem && <p className="problem" role="alert">{problem}</p>}
            <button className="primary" disabled={busy}>{busy ? "Signing in" : "Sign in"}</button>
          </form>
        ) : (
          <form onSubmit={sendCode}>
            <p className="lede">{methodNames[method]?.hint ?? "Enter your verification code."}</p>
            {methods.length > 1 && (
              <div className="segmented" role="radiogroup" aria-label="Where the code comes from">
                {methods.map((m) => (
                  <button type="button" key={m} role="radio" aria-checked={m === method} className={m === method ? "on" : ""} onClick={() => { setMethod(m); setCode(""); }}>
                    {methodNames[m].label}
                  </button>
                ))}
              </div>
            )}
            <label>
              Code
              <input
                autoFocus
                className="code"
                inputMode={method === "otp" ? "text" : "numeric"}
                autoComplete="one-time-code"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                required
              />
            </label>
            {problem && <p className="problem" role="alert">{problem}</p>}
            <button className="primary" disabled={busy}>{busy ? "Checking" : "Finish signing in"}</button>
            <button type="button" className="quiet" onClick={() => { setMethods(null); setProblem(""); setCode(""); }}>
              Use a different account
            </button>
          </form>
        )}
        {backend.demo && (
          <p className="demo-note">
            Demo data. This tab is not connected to VRChat: type anything, then any 6 digits.
          </p>
        )}
      </section>
    </main>
  );
}
