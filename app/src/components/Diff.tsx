import type { DiffOp } from '../ipc'

/** A word diff: deletions struck through, insertions highlighted (docs/PRODUCT.md §5). */
export default function Diff({ ops }: { ops: DiffOp[] }) {
  return (
    <p className="diff" aria-label="Changes">
      {ops.map((op, i) => {
        if ('delete' in op) return <del key={i}>{op.delete}</del>
        if ('insert' in op) return <ins key={i}>{op.insert}</ins>
        return <span key={i}>{op.equal}</span>
      })}
    </p>
  )
}
