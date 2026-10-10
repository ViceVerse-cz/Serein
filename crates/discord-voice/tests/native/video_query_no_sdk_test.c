/* An installed FFmpeg encoder without query SDK support remains Unknown. */
#include <assert.h>
#include <stdio.h>
#include <libavcodec/avcodec.h>
#ifdef NDEBUG
#error Driver fixtures require assertions
#endif
int serein_video_query(int backend, int codec);
const AVCodec *avcodec_find_encoder_by_name(const char *name)
{
    static const AVCodec encoder = {0};
    assert(name);
    return &encoder;
}
int main(void)
{
    for (int codec = 0; codec < 3; codec++) {
        for (int backend = 1; backend <= 4; backend++) {
            if (backend != 2) assert(serein_video_query(backend, codec) == -1);
        }
    }
    puts("Missing query SDK: installed FFmpeg modules stay inconclusive");
    return 0;
}
