# npm corpus tools

`corpus/` holds the 24 npm programs (pinned in `corpus/versions.txt`) used by the jit-coverage, route-all and profile-corpus workflows. `measure.sh` builds and runs them with thaw-js and reports build/run/stdout-vs-Node rows as TSV. The directory keeps its historical name because the workflows reference it by path.
