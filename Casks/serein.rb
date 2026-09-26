cask "serein" do
  version "1.0.0-nightly.20260926.47"
  sha256 "73dee00ab03c13b6d5877af95a3d5551a2abf0a262490c6caf78411a17bc1d2f"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
