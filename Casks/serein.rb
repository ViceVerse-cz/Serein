cask "serein" do
  version "1.0.0-nightly.20261014.58"
  sha256 "efbaf7adac3075c639249d387aa419bf44fe39ab84c079e4080cd6b0a228ed65"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  # Frozen: this cask moved to the ViceVerse-cz/homebrew-serein tap and is no longer updated here.
  deprecate! date: "2026-10-16",
             because: "moved to its own tap; run `brew untap ViceVerse-cz/serein && brew tap ViceVerse-cz/serein`"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
