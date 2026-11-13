import { footerLine } from '../state/popup-view'
import type { RewriteOutcome } from '../ipc'

/**
 * The line under a result: `<tier> · <cost> · <balance line>` on publik API, the provider line
 * alone otherwise (`Your own key · <model>`, `Local · <model>`).
 */
export default function Footer({
  outcome,
  publik,
  balanceLine,
}: {
  outcome: RewriteOutcome
  publik: boolean
  balanceLine: string | null
}) {
  return <p className="footer">{footerLine(outcome, publik, balanceLine)}</p>
}
