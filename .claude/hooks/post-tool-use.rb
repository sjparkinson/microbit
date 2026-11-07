#!/usr/bin/env ruby
# PostToolUse hook on Edit and Write. Lints the file that changed and, on a
# problem, exits 2 so stderr goes back to Claude to fix.
#
#   .rs  rustfmt --check, then the host clippy. Files in board/ or firmware/
#        also get clippy on both thumb targets, since board/src/hw.rs and each
#        firmware's real main() only compile there.
#   .rb  ruby -wc
#   .github/workflows/*.yml  gh actions-lock --verify-local (lockfile coverage)

require "json"
require "open3"

BOARDS = [
  %w[v1 thumbv6m-none-eabi],
  %w[v2 thumbv7em-none-eabihf]
].freeze

def run(*cmd, chdir:)
  out, status = Open3.capture2e(*cmd, chdir: chdir)
  status.success? ? nil : "$ #{cmd.join(' ')}\n#{out.lines.last(80).join}"
end

# The crate a file belongs to, if it's one that builds for a micro:bit.
def device_package(relative)
  return "board" if relative.start_with?("board/")

  relative[%r{\Afirmware/([^/]+)/}, 1]
end

def rust(path, dir)
  relative = path.delete_prefix("#{dir}/")
  fmt = run("rustfmt", "--edition", "2024", "--check", "--color", "never", path, chdir: dir)
  return fmt if fmt

  host = run("cargo", "clippy", "--workspace", "--all-targets", "--locked", "-q", "--", "-D", "warnings", chdir: dir)
  return host if host

  package = device_package(relative)
  return nil unless package

  BOARDS.each do |board, target|
    problem = run("cargo", "clippy", "-q", "--locked", "--release", "--package", package,
                  "--target", target, "--features", board, "--", "-D", "warnings", chdir: dir)
    return problem if problem
  end
  nil
end

input = JSON.parse($stdin.read)
path = input.dig("tool_input", "file_path").to_s
exit 0 if path.empty? || !File.file?(path)

# The checkout the file lives in, so edits in a worktree lint that worktree.
top, status = Open3.capture2("git", "-C", File.dirname(path), "rev-parse", "--show-toplevel")
dir = status.success? ? top.chomp : Dir.pwd
problem =
  if path.start_with?("#{dir}/.github/workflows/") && path.end_with?(".yml", ".yaml")
    run("gh", "actions-lock", "--verify-local", chdir: dir)
  else
    case File.extname(path)
    when ".rs" then rust(path, dir)
    when ".rb" then run("ruby", "-wc", path, chdir: dir)
    end
  end

exit 0 unless problem

warn("Lint problems after editing #{path}:\n#{problem}")
exit 2
