<div align="center">
  <img src="logo.png" alt="the decomp-dj logo, which is a stylized pixel-art turntable with the two discs and some buttons resembling the coverage and parity graphs">

# decomp-dj

### Streamlined dynamic analysis and playtesting for retro game decomps
</div>

* Store and edit git-friendly sequential and branching input replays.
* Build caches of incremental memory snapshots on developer machines to navigate to any game state in real-time.
* Avoid having to commit massive blobs of (potentially copyright-violating) reference savestates.
* Automate regression testing on decomps by running them side-by-side with the original game.
* Track and visualize coverage and decomp parity by function and block.
* Cache xrefs of replay frames to executed code blocks, allowing for instantaneous tracing of unknown functions.
* MCP-enabled, accelerating agentic workflows by enabling them to:
  * jump, step, and trace through metadata-rich gameplay segments
  * fuzz and test game logic
  * decomp with more accurate comments and symbol names
  * learn to play the game (or even create whole new replay segments on your behalf)
  * catch exactly when and why a decomp loses parity with the original game
  * stop asking you to pause the game in specific states every two minutes