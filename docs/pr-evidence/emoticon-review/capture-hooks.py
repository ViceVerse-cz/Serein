from pathlib import Path
import sys
root=Path(sys.argv[1])
if len(sys.argv)>2 and sys.argv[2]=='restore':
 for f in ['crates/ui/src/lib.rs','apps/desktop/examples/profile_preview.rs']:
  backup=root/(f+'.emoticon-review-backup')
  (root/f).write_bytes(backup.read_bytes()); backup.unlink()
 sys.exit()
for f in ['crates/ui/src/lib.rs','apps/desktop/examples/profile_preview.rs']:
 if (root/(f+'.emoticon-review-backup')).exists():
  raise SystemExit('Restore the existing capture hook before applying it again.')
for f in ['crates/ui/src/lib.rs','apps/desktop/examples/profile_preview.rs']:
 p=root/f
 (root/(f+'.emoticon-review-backup')).write_bytes(p.read_bytes())
p=root/'crates/ui/src/lib.rs'
s=p.read_text()+r'''
#[cfg(feature = "demo")]
impl MessagingUi {
    pub fn preview_emoticon_review(&mut self, state: &mut State) {
        let channel = state.selected.unwrap();
        let mut message = synthetic_message(state, channel);
        let id = message.id;
        message.content = "Original synthetic message".into();
        state.timeline.clear();
        state.timeline.seed_cache(vec![message]).unwrap();
        self.convert_emoticons = true;
        self.editing = Some((channel, id, "Hello :)".into()));
        let ctx = egui::Context::default();
        for frame in 0..2 {
            let events = if frame == 1 { vec![egui::Event::Key {
                key: egui::Key::Enter, physical_key: None, pressed: true,
                repeat: false, modifiers: egui::Modifiers::NONE,
            }] } else { vec![] };
            let mut commands = vec![];
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                self.composer(ui, state, channel, &ctx, &mut commands);
            }).drop_without_applying_deltas();
            if frame == 1 {
                let [Command::Edit { request, .. }] = commands.as_slice() else { panic!("synthetic edit submission") };
                state.apply(client_core::Envelope {
                    generation: state.generation,
                    event: client_core::Event::Edited {
                        channel, message: id, request: *request,
                        result: Ok(state.timeline.get(id).unwrap().clone()),
                    },
                });
            }
        }
    }
}
#[cfg(feature = "demo")]
fn synthetic_message(state: &State, channel: Id) -> model::Message {
    let mut message = state.timeline.iter().next().unwrap().clone();
    message.channel = channel;
    message.author = state.user.clone().unwrap();
    message.attachments.clear(); message.embeds.clear(); message.reactions = Some(vec![]);
    message
}
'''
p.write_text(s)
p=root/'apps/desktop/examples/profile_preview.rs';s=p.read_text()
s=s.replace('"profile"\n\t\t\t| "stickers"','"profile"\n\t\t\t| "emoticon-edit"\n\t\t\t| "stickers"',1)
s=s.replace('''\t\t\t} else if page.starts_with("server") {''','''\t\t\t} else if page == "emoticon-edit" {
                messaging.preview_emoticon_review(&mut state);
            } else if page.starts_with("server") {''',1)
p.write_text(s)
